//! Following other users (found by exact username only) and their profiles. A private user's
//! profile shows only their username; everything else needs them to be public.

use migration::{Alias, Expr, ExprTrait, JoinType};
use sea_orm::{ConnectionTrait, DatabaseConnection, EntityTrait, QueryResult, QuerySelect, RelationTrait, Statement};
use tonic::{Request, Response, Status};

use crate::data;
use crate::interceptor::auth::UserPermissions;
use crate::proto::friend_server::{Friend, FriendServer};
use crate::proto::profile_request::User as ProfileUser;
use crate::proto::{
    FriendRequest, PaginateQuery, PaginateReply, ProfileReply, ProfileRequest, UserSummariesReply, UserSummary,
};
use crate::util::auth::Authorize;
use crate::util::db::DatabaseRequest;
use crate::util::stats;

fn internal<E: ToString>(e: E) -> Status {
    Status::internal(e.to_string())
}

async fn query(db: &DatabaseConnection, sql: &str, values: Vec<sea_orm::Value>) -> Result<Vec<QueryResult>, Status> {
    db.query_all_raw(Statement::from_sql_and_values(db.get_database_backend(), sql, values))
        .await
        .map_err(internal)
}

/// Get a user by their ID
#[rustfmt::skip]
pub async fn get_user_by_id(db: &DatabaseConnection, user_id: i32) -> Result<data::user::Full, Status> {
    let following_alias = Alias::new("following");
    let followers_alias = Alias::new("followers");

    let user = entity::user::Entity::find_by_id(user_id)
        .join_as(JoinType::LeftJoin, entity::friend::Relation::User1.def().rev(), following_alias.clone())
        .join_as(JoinType::LeftJoin, entity::friend::Relation::User2.def().rev(), followers_alias.clone())
        .column_as(Expr::col((following_alias, entity::friend::Column::UserId)).count(), "count_following")
        .column_as(Expr::col((followers_alias, entity::friend::Column::FriendId)).count(), "count_followers")
        .group_by(entity::user::Column::Id)
        .into_model::<data::user::Full>()
        .one(db)
        .await
        .map_err(|e| Status::internal(e.to_string()))?
        .ok_or(Status::not_found("User not found"))?;

    Ok(user)
}

/// The user with exactly this username: case-sensitive first, else case-insensitive when that
/// matches only one user.
async fn find_by_username(db: &DatabaseConnection, username: &str) -> Result<entity::user::Model, Status> {
    let username = username.trim();
    let rows = query(
        db,
        r#"SELECT id, username FROM "user" WHERE lower(username) = lower($1) ORDER BY (username = $1) DESC, id LIMIT 2"#,
        vec![username.into()],
    )
    .await?;
    let matches: Vec<(i32, String)> = rows
        .iter()
        .map(|row| Ok((row.try_get("", "id")?, row.try_get("", "username")?)))
        .collect::<Result<_, sea_orm::DbErr>>()
        .map_err(internal)?;
    let id = match matches.as_slice() {
        [(id, _)] => Some(*id),
        [(id, name), _] if name == username => Some(*id),
        _ => None,
    };
    match id {
        Some(id) => find_by_id(db, id).await,
        None => Err(Status::not_found("No user with that username")),
    }
}

async fn find_by_id(db: &DatabaseConnection, id: i32) -> Result<entity::user::Model, Status> {
    entity::user::Entity::find_by_id(id)
        .one(db)
        .await
        .map_err(internal)?
        .ok_or(Status::not_found("User not found"))
}

async fn target(db: &DatabaseConnection, req: &FriendRequest) -> Result<entity::user::Model, Status> {
    match req.username.as_deref() {
        Some(username) if !username.trim().is_empty() => find_by_username(db, username).await,
        _ => find_by_id(db, req.user_id).await,
    }
}

/// Columns of a `UserSummary` for `u`, seen by user `$1`.
const SUMMARY_COLUMNS: &str = r#"
    u.id, u.username, u.is_private,
    EXISTS (SELECT 1 FROM friend mine WHERE mine.user_id = $1 AND mine.friend_id = u.id) AS followed_by_me,
    EXISTS (SELECT 1 FROM friend theirs WHERE theirs.user_id = u.id AND theirs.friend_id = $1) AS follows_me
"#;

fn summary(row: &QueryResult) -> Result<UserSummary, Status> {
    let since: Option<chrono::NaiveDateTime> = row.try_get("", "since").ok();
    Ok(UserSummary {
        id: row.try_get("", "id").map_err(internal)?,
        username: row.try_get("", "username").map_err(internal)?,
        is_private: row.try_get("", "is_private").map_err(internal)?,
        followed_by_me: row.try_get("", "followed_by_me").map_err(internal)?,
        follows_me: row.try_get("", "follows_me").map_err(internal)?,
        since: since.map_or(0, |at| at.and_utc().timestamp_millis()),
    })
}

/// `user_id` as seen by `viewer_id`.
async fn summary_of(db: &DatabaseConnection, viewer_id: i32, user_id: i32) -> Result<UserSummary, Status> {
    let rows = query(
        db,
        &format!(r#"SELECT {SUMMARY_COLUMNS} FROM "user" u WHERE u.id = $2"#),
        vec![viewer_id.into(), user_id.into()],
    )
    .await?;
    summary(rows.first().ok_or(Status::not_found("User not found"))?)
}

/// The people the logged-in user follows (`following`) or who follow them, most recent first.
async fn index(request: Request<PaginateQuery>, following: bool) -> Result<Response<UserSummariesReply>, Status> {
    let db = request.db()?;
    let logged_in = request.authorize()?;
    let req = request.get_ref();
    let per_page = req.per_page.unwrap_or(20).clamp(1, 50);

    // `f.user_id` follows `f.friend_id`.
    let (me, them) = if following {
        ("f.user_id", "f.friend_id")
    } else {
        ("f.friend_id", "f.user_id")
    };

    let total: i64 = query(
        db,
        &format!("SELECT COUNT(*) AS total FROM friend f WHERE {me} = $1"),
        vec![logged_in.id.into()],
    )
    .await?
    .first()
    .map(|row| row.try_get("", "total"))
    .transpose()
    .map_err(internal)?
    .unwrap_or_default();
    let total = total as u64;
    let max_page = total.div_ceil(per_page).saturating_sub(1);
    let page = std::cmp::min(req.page.unwrap_or(0), max_page);

    let rows = query(
        db,
        &format!(
            r#"
            SELECT {SUMMARY_COLUMNS}, f.created_at AS since
            FROM friend f
            JOIN "user" u ON u.id = {them}
            WHERE {me} = $1
            ORDER BY f.created_at DESC, u.id
            LIMIT $2 OFFSET $3
            "#
        ),
        vec![
            logged_in.id.into(),
            (per_page as i64).into(),
            ((page * per_page) as i64).into(),
        ],
    )
    .await?;

    Ok(Response::new(UserSummariesReply {
        pagination: Some(PaginateReply {
            page,
            per_page,
            max_page,
            total,
        }),
        items: rows.iter().map(summary).collect::<Result<_, _>>()?,
    }))
}

#[derive(Debug, Default)]
pub struct FriendController;

#[tonic::async_trait]
impl Friend for FriendController {
    async fn following(&self, request: Request<PaginateQuery>) -> Result<Response<UserSummariesReply>, Status> {
        index(request, true).await
    }

    async fn followers(&self, request: Request<PaginateQuery>) -> Result<Response<UserSummariesReply>, Status> {
        index(request, false).await
    }

    async fn follow(&self, request: Request<FriendRequest>) -> Result<Response<UserSummary>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?;
        let user = target(db, request.get_ref()).await?;
        if user.id == logged_in.id {
            return Err(Status::invalid_argument("You can't follow yourself"));
        }

        db.execute_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            "INSERT INTO friend (user_id, friend_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            [logged_in.id.into(), user.id.into()],
        ))
        .await
        .map_err(internal)?;

        Ok(Response::new(summary_of(db, logged_in.id, user.id).await?))
    }

    async fn unfollow(&self, request: Request<FriendRequest>) -> Result<Response<UserSummary>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?;
        let user = target(db, request.get_ref()).await?;

        db.execute_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            "DELETE FROM friend WHERE user_id = $1 AND friend_id = $2",
            [logged_in.id.into(), user.id.into()],
        ))
        .await
        .map_err(internal)?;

        Ok(Response::new(summary_of(db, logged_in.id, user.id).await?))
    }

    async fn profile(&self, request: Request<ProfileRequest>) -> Result<Response<ProfileReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?;
        let req = request.get_ref();

        let user = match &req.user {
            Some(ProfileUser::Username(username)) => find_by_username(db, username).await?,
            Some(ProfileUser::UserId(id)) => find_by_id(db, *id).await?,
            None => logged_in.clone(),
        };
        let is_me = user.id == logged_in.id;
        let visible = is_me || !user.is_private;
        let counts = get_user_by_id(db, user.id).await?;

        Ok(Response::new(ProfileReply {
            user: Some(summary_of(db, logged_in.id, user.id).await?),
            count_following: counts.count_following,
            count_followers: counts.count_followers,
            member_since: user.created_at.and_utc().timestamp_millis(),
            is_me,
            visible,
            stats: if visible {
                Some(stats::detailed(db, &user, req.utc_offset_minutes, is_me).await?)
            } else {
                None
            },
        }))
    }
}

crate::export_service!(FriendServer, FriendController, auth = UserPermissions::USER);
