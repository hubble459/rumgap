use derive_more::Deref;
use hmac::{Hmac, Mac};
use jwt::{SignWithKey, VerifyWithKey};
use lazy_static::lazy_static;
use sea_orm::{EntityTrait, DatabaseConnection};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

use actix_web::{error::{ErrorUnauthorized, ErrorNotFound, ErrorInternalServerError}, Error, FromRequest, web};
use sha2::Sha256;

#[derive(Serialize, Deserialize)]
pub struct Token {
    pub id: i32,
}

lazy_static! {
    static ref SECRET_KEY: Hmac<Sha256> = Hmac::new_from_slice(b"bUHhhHH#!bU@NkNUnK12").unwrap();
}

pub fn sign(id: i32) -> Result<String, jwt::Error> {
    Token { id }.sign_with_key(&SECRET_KEY.clone())
}

#[derive(Deref)]
pub struct User(pub entity::user::Model);

bitflags! {
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

pub struct AuthService {
    pub user: User,
}

impl FromRequest for AuthService {
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self, Self::Error>>>>;

    fn from_request(
        req: &actix_web::HttpRequest,
        _payload: &mut actix_web::dev::Payload,
    ) -> Self::Future {
        let req = req.clone();
        Box::pin(async move {
            let auth = req
                .headers()
                .get("Authorization")
                .ok_or(ErrorUnauthorized("Missing Bearer token"))?
                .to_str()
                .map_err(|_| ErrorUnauthorized("Missing Bearer token"))?;

            if auth.starts_with("Bearer ") {
                let token = auth.split_once("Bearer ").unwrap().1.trim();
                let Token { id } = token
                    .verify_with_key(&SECRET_KEY.clone())
                    .map_err(|e| ErrorUnauthorized(e.to_string()))?;

                let conn: &web::Data<DatabaseConnection> = req.app_data().unwrap();
                let user = entity::user::Entity::find_by_id(id)
                    .one(conn.as_ref())
                    .await
                    .map_err(ErrorInternalServerError)?
                    .ok_or(ErrorNotFound(""))?;

                Ok(Self { user: User(user) })
            } else {
                Err(ErrorUnauthorized("Missing Bearer token"))
            }
        })
    }

    fn extract(req: &actix_web::HttpRequest) -> Self::Future {
        Self::from_request(req, &mut actix_web::dev::Payload::None)
    }
}