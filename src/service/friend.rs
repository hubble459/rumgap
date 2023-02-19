use migration::{Alias, Expr, JoinType};
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter, QuerySelect, RelationTrait,
};
use tonic::{Request, Response, Status};

use crate::data;
use crate::interceptor::auth::{LoggedInUser, UserPermissions};
use crate::proto::friend_server::{Friend, FriendServer};
use crate::proto::{
    FriendRequest, PaginateQuery, PaginateReply, UserFullReply, UserReply, UsersReply,
};
use crate::util::verify;

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

/// Get all following or followers
/// [following=true] Index Following 
/// [following=false] Index Followers 
async fn index(
    request: Request<PaginateQuery>,
    following: bool,
) -> Result<Response<UsersReply>, Status> {
    let db = request.extensions().get::<DatabaseConnection>().unwrap();
    let logged_in = request.extensions().get::<LoggedInUser>().unwrap();
    let req = request.get_ref();
    let per_page = req.per_page.unwrap_or(10).clamp(1, 50);

    let paginate = entity::user::Entity::find()
        .join(
            JoinType::RightJoin,
            if following {
                entity::friend::Relation::User1.def().rev()
            } else {
                entity::friend::Relation::User2.def().rev()
            },
        )
        .filter(
            entity::friend::Column::UserId
                .eq(logged_in.id)
                .or(entity::friend::Column::FriendId.eq(logged_in.id)),
        )
        .paginate(db, per_page);

    // Get max page and total items
    let amount = paginate
        .num_items_and_pages()
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

    let max_page = if amount.number_of_pages == 0 {
        0
    } else {
        amount.number_of_pages - 1
    };

    let page = req.page.unwrap_or(0).clamp(0, max_page);

    // Get items from page
    let items = paginate
        .fetch_page(page)
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

    Ok(Response::new(UsersReply {
        pagination: Some(PaginateReply {
            page,
            per_page,
            max_page,
            total: amount.number_of_items,
        }),
        items: items
            .into_iter()
            .map(|user| UserReply {
                id: user.id,
                username: user.username,
                email: user.email,
                permissions: user.permissions as i32,
                preferred_hostnames: user.preferred_hostnames,
                device_ids: user.device_ids,
                created_at: user.created_at.timestamp_millis(),
                updated_at: user.updated_at.timestamp_millis(),
            })
            .collect(),
    }))
}

#[derive(Debug, Default)]
pub struct MyFriend {}

#[tonic::async_trait]
impl Friend for MyFriend {
    /// Get all following from logged in user
    async fn following(
        &self,
        request: Request<PaginateQuery>,
    ) -> Result<Response<UsersReply>, Status> {
        index(request, true).await
    }

    /// Get all followers from logged in user
    async fn followers(
        &self,
        request: Request<PaginateQuery>,
    ) -> Result<Response<UsersReply>, Status> {
        index(request, false).await
    }

    /// Follow a user
    async fn follow(
        &self,
        request: Request<FriendRequest>,
    ) -> Result<Response<UserFullReply>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let logged_in = request.extensions().get::<LoggedInUser>().unwrap();
        let req = request.get_ref();

        let friend = entity::friend::ActiveModel {
            user_id: ActiveValue::Set(logged_in.id),
            friend_id: ActiveValue::Set(req.user_id),
            ..Default::default()
        };

        let friend = friend.insert(db).await.map_err(|e| {
            if verify::is_conflict(&e) {
                return Status::already_exists("Already following!");
            }
            Status::internal(e.to_string())
        })?;

        let full_user = get_user_by_id(db, friend.friend_id).await?;
        Ok(Response::new(full_user.into()))
    }

    /// Unfollow a user
    async fn unfollow(
        &self,
        request: Request<FriendRequest>,
    ) -> Result<Response<UserFullReply>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let logged_in = request.extensions().get::<LoggedInUser>().unwrap();
        let req = request.get_ref();

        let friend = entity::friend::Entity::delete_by_id((logged_in.id, req.user_id))
            .exec(db)
            .await
            .map_err(|e| Status::aborted(e.to_string()))?;

        if friend.rows_affected == 0 {
            Err(Status::not_found("Friend not found, so did not unfollow"))
        } else {
            let full_user = get_user_by_id(db, req.user_id).await?;
            Ok(Response::new(full_user.into()))
        }
    }
}

crate::export_service!(FriendServer, MyFriend, auth = UserPermissions::USER);
