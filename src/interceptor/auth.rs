use derive_more::Deref;
use hmac::{Hmac, Mac};
use jwt::{SignWithKey, VerifyWithKey};
use sea_orm::{DatabaseConnection, EntityTrait};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tonic::service::Interceptor;
use tonic::{Request, Status};

lazy_static! {
    static ref SECRET_KEY: Hmac<Sha256> = Hmac::new_from_slice(b"bUHhhHH#!bU@NkNUnK12").unwrap();
}

#[derive(Serialize, Deserialize)]
pub struct Token {
    pub id: i32,
}

pub fn sign(id: i32) -> Result<String, jwt::Error> {
    Token { id }.sign_with_key(&SECRET_KEY.clone())
}

#[derive(Deref)]
pub struct LoggedInUser(pub entity::user::Model);

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct UserPermissions: u32 {
        const USER = 0b00000001;
        const MOD = 0b00000010;
        const ADMIN = 0b00000100;
    }
}

impl LoggedInUser {
    pub fn has_permission(&self, permission: UserPermissions) -> bool {
        (UserPermissions::from_bits(self.0.permissions as u32).unwrap() & permission) == permission
    }
}

pub async fn check_auth(mut req: Request<()>) -> Result<Request<()>, Status> {
    let db = req.extensions().get::<DatabaseConnection>().unwrap();
    let bearer_token = req.metadata().get("authorization");

    if let Some(bearer_token) = bearer_token {
        let bearer_token = bearer_token
            .to_str()
            .map_err(|_| Status::unauthenticated("Bearer token is invalid"))?;
        let token = bearer_token.strip_prefix("Bearer ");
        if let Some(token) = token {
            let Token { id: user_id } = token
                .verify_with_key(&SECRET_KEY.clone())
                .map_err(|e| Status::unauthenticated(e.to_string()))?;

            let user = entity::user::Entity::find_by_id(user_id)
                .one(db)
                .await
                .map_err(|_e| Status::internal("Bearer token is invalid"))?
                .ok_or(Status::unauthenticated(
                    "User belonging to this token does not exists anymore",
                ))?;

            req.extensions_mut().insert(LoggedInUser(user));
        } else {
            return Err(Status::unauthenticated("Bearer token is invalid"));
        }
    }

    Ok(req)
}

pub fn logged_in(perms: UserPermissions) -> LoggedInCheck {
    LoggedInCheck { perms }
}

#[derive(Clone)]
pub struct LoggedInCheck {
    perms: UserPermissions,
}

impl Interceptor for LoggedInCheck {
    fn call(&mut self, req: tonic::Request<()>) -> Result<tonic::Request<()>, Status> {
        let user = req.extensions().get::<LoggedInUser>();

        match user {
            Some(user) => {
                if user.has_permission(self.perms) {
                    Ok(req)
                } else {
                    Err(Status::permission_denied("You are missing permissions to make this call"))
                }
            },
            None => Err(Status::unauthenticated(
                "You need to be logged in to make this request!",
            )),
        }
    }
}
