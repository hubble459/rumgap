//! Admin-only user management.

use sea_orm::{ActiveModelTrait, ActiveValue, EntityTrait, IntoActiveModel};
use tonic::{Request, Response, Status};

use crate::interceptor::auth::UserPermissions;
use crate::proto::admin_server::{Admin, AdminServer};
use crate::proto::{AdminCreateUserRequest, AdminDeleteUserRequest, AdminSetPasswordRequest, Empty, UserReply};
use crate::util::auth::Authorize;
use crate::util::db::DatabaseRequest;
use crate::util::{argon, verify};

fn internal<E: ToString>(e: E) -> Status {
    Status::internal(e.to_string())
}

#[derive(Debug, Default)]
pub struct AdminController;

#[tonic::async_trait]
impl Admin for AdminController {
    /// Create a user
    async fn create_user(&self, request: Request<AdminCreateUserRequest>) -> Result<Response<UserReply>, Status> {
        let db = request.db()?;
        let req = request.get_ref();

        let mut permissions = UserPermissions::USER;
        if req.admin {
            permissions |= UserPermissions::ADMIN;
        }

        let user = entity::user::ActiveModel {
            username: ActiveValue::Set(verify::username(&req.username)?),
            email: ActiveValue::Set(verify::email(&req.email)?),
            password_hash: ActiveValue::Set(argon::encrypt(&verify::password(&req.password)?)?),
            permissions: ActiveValue::Set(permissions.bits() as i16),
            preferred_hostnames: ActiveValue::Set(vec![]),
            device_ids: ActiveValue::Set(vec![]),
            ..Default::default()
        };

        let user = user.insert(db).await.map_err(|e| {
            if verify::is_conflict(&e) {
                return Status::already_exists("Username or email already in use");
            }
            internal(e)
        })?;

        Ok(Response::new(UserReply {
            id: user.id,
            username: user.username,
            email: user.email,
            permissions: user.permissions as i32,
            preferred_hostnames: user.preferred_hostnames,
            device_ids: user.device_ids,
            created_at: user.created_at.and_utc().timestamp_millis(),
            updated_at: user.updated_at.and_utc().timestamp_millis(),
        }))
    }

    /// Delete a user (reading, offsets and friends cascade)
    async fn delete_user(&self, request: Request<AdminDeleteUserRequest>) -> Result<Response<Empty>, Status> {
        let logged_in = request.authorize()?;
        let db = request.db()?;
        let req = request.get_ref();

        if logged_in.id == req.id {
            return Err(Status::failed_precondition("You cannot delete your own account"));
        }

        let result = entity::user::Entity::delete_by_id(req.id)
            .exec(db)
            .await
            .map_err(internal)?;

        if result.rows_affected == 0 {
            return Err(Status::not_found("User not found"));
        }

        Ok(Response::new(Empty::default()))
    }

    /// Set a user's password
    async fn set_password(&self, request: Request<AdminSetPasswordRequest>) -> Result<Response<Empty>, Status> {
        let db = request.db()?;
        let req = request.get_ref();

        let user = entity::user::Entity::find_by_id(req.id)
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("User not found"))?;

        let mut active_user = user.into_active_model();
        active_user.password_hash = ActiveValue::Set(argon::encrypt(&verify::password(&req.password)?)?);
        active_user.update(db).await.map_err(internal)?;

        Ok(Response::new(Empty::default()))
    }
}

crate::export_service!(AdminServer, AdminController, auth = UserPermissions::ADMIN);
