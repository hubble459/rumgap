use derive_more::Deref;
use hmac::{Hmac, Mac};
use jwt::{SignWithKey, VerifyWithKey};
use sea_orm::{DatabaseConnection, EntityTrait};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
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
pub struct User(pub entity::user::Model);

bitflags! {
    #[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Copy, Clone)]
    pub struct UserPermissions: u32 {
        const USER = 0b00000001;
        const MOD = 0b00000010;
        const ADMIN = 0b00000100;
    }
}

impl User {
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

            req.extensions_mut().insert(User(user));
        } else {
            return Err(Status::unauthenticated("Bearer token is invalid"));
        }
    }

    Ok(req)
}
