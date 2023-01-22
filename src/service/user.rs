use migration::{Alias, Expr, JoinType};
use sea_orm::{DatabaseConnection, EntityTrait, PaginatorTrait, QuerySelect, RelationTrait};
use tonic::{Request, Response, Status};

use crate::proto::user_server::{User, UserServer};
use crate::proto::{
    Id, PaginateQuery, PaginateReply, UserRegisterRequest, UserReply, UserRequest, UserTokenReply,
    UsersReply,
};

#[rustfmt::skip]
pub async fn get_user_by_id(db: &DatabaseConnection, user_id: i32) -> Result<data::user::Full, Status> {
    let following_alias = Alias::new("following");
    let followers_alias = Alias::new("followers");

    let user = entity::user::Entity::find_by_id(user_id)
        .join_as(JoinType::LeftJoin, entity::friend::Relation::User1.def().rev(), following_alias.clone())
        .join_as(JoinType::LeftJoin, entity::friend::Relation::User2.def().rev(), followers_alias.clone())
        .column_as(Expr::col((following_alias, entity::friend::Column::Id)).count(), "count_following")
        .column_as(Expr::col((followers_alias, entity::friend::Column::Id)).count(), "count_followers")
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
    async fn get(&self, request: Request<Id>) -> Result<Response<UserReply>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();

        let req = request.get_ref();

        let user = entity::user::Entity::find_by_id(req.id)
            .one(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or(Status::not_found("User not found"))?;

        Ok(Response::new(UserReply {
            id: user.id,
            username: user.username,
            email: user.email,
            permissions: user.permissions as i32,
            created_at: user.created_at.timestamp_millis(),
            updated_at: user.updated_at.timestamp_millis(),
        }))
    }

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

        let page = req.page.unwrap_or(0).clamp(0, amount.number_of_pages - 1);

        // Get items from page
        let items = paginate
            .fetch_page(page)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(UsersReply {
            pagination: Some(PaginateReply {
                page,
                per_page,
                max_page: amount.number_of_pages - 1,
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

    async fn register(
        &self,
        request: Request<UserRegisterRequest>,
    ) -> Result<Response<UserTokenReply>, Status> {
        unimplemented!();
        // let req = request.into_inner();
        // Ok(Response::new(UserReply {
        //     id: 1,
        //     username: format!("Hello {}!", req.username),
        // }))
    }

    async fn login(
        &self,
        request: Request<UserRequest>,
    ) -> Result<Response<UserTokenReply>, Status> {
        unimplemented!();

        // let req = request.into_inner();
        // Ok(Response::new(UserReply {
        //     id: 0,
        //     username: req.username,
        // }))
    }
}

pub fn server() -> UserServer<MyUser> {
    UserServer::new(MyUser::default())
}
