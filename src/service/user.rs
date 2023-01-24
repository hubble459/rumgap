use migration::{Alias, Expr, JoinType};
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter, QuerySelect, RelationTrait,
};
use tonic::{Request, Response, Status};

use crate::data;
use crate::interceptor::auth::sign;
use crate::proto::user_request::Identifier;
use crate::proto::user_server::{User, UserServer};
use crate::proto::{
    Id, PaginateQuery, PaginateReply, UserFullReply, UserRegisterRequest, UserReply, UserRequest,
    UserTokenReply, UsersReply,
};
use crate::util::{argon, verify};

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

#[derive(Debug, Default)]
pub struct MyUser {}

#[tonic::async_trait]
impl User for MyUser {
    /// Get a single user
    async fn get(&self, request: Request<Id>) -> Result<Response<UserFullReply>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let req = request.get_ref();
        let full_user = get_user_by_id(db, req.id).await?;
        Ok(Response::new(full_user.into()))
    }

    /// Get paginated users
    async fn index(&self, request: Request<PaginateQuery>) -> Result<Response<UsersReply>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let req = request.get_ref();
        let per_page = req.per_page.unwrap_or(10).clamp(1, 50);
        let paginate = entity::user::Entity::find().paginate(db, per_page);

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
                    created_at: user.created_at.timestamp_millis(),
                    updated_at: user.updated_at.timestamp_millis(),
                })
                .collect(),
        }))
    }

    /// Register a new account
    async fn register(
        &self,
        request: Request<UserRegisterRequest>,
    ) -> Result<Response<UserTokenReply>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let req = request.get_ref();

        let user = entity::user::ActiveModel {
            username: ActiveValue::Set(verify::username(&req.username)?),
            email: ActiveValue::Set(verify::email(&req.email)?),
            password_hash: ActiveValue::Set(argon::encrypt(&verify::password(&req.password)?)?),
            ..Default::default()
        };

        let user = user.insert(db).await.map_err(|e| {
            if verify::is_conflict(&e) {
                return Status::already_exists("Username or email already in use");
            }
            Status::internal(e.to_string())
        })?;

        let full_user = get_user_by_id(db, user.id).await?;
        let token = sign(full_user.id).map_err(|e| Status::aborted(e.to_string()))?;
        Ok(Response::new(UserTokenReply {
            token,
            user: Some(full_user.into()),
        }))
    }

    /// Log in with username/email and password
    async fn login(
        &self,
        request: Request<UserRequest>,
    ) -> Result<Response<UserTokenReply>, Status> {
        let error = "Username and password mismatch";
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let req = request.get_ref();

        let identifier = req.identifier.as_ref().unwrap();

        let filter = match identifier {
            Identifier::Username(username) => {
                entity::user::Column::Username.eq(username.to_ascii_lowercase())
            }
            Identifier::Email(email) => entity::user::Column::Email.eq(email.to_ascii_lowercase()),
        };

        let user = entity::user::Entity::find()
            .filter(filter)
            .one(db)
            .await
            .map_err(|e| Status::aborted(e.to_string()))?
            .ok_or(Status::not_found(error))?;

        argon::verify(&user.password_hash, &req.password)?;

        let full_user = get_user_by_id(db, user.id).await?;
        let token = sign(full_user.id).map_err(|e| Status::aborted(e.to_string()))?;
        Ok(Response::new(UserTokenReply {
            token,
            user: Some(full_user.into()),
        }))
    }
}

crate::export_service!(UserServer, MyUser);
