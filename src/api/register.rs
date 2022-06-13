use crate::api::login::Token;
use crate::auth::{User, KEY_BYTES};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use chrono::Utc;
use entity::user::ActiveModel as ActiveUser;
use entity::user::Entity as UserTable;
use hmac::{Hmac, Mac};
use jwt::SignWithKey;
use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::{
    serde::{Deserialize, Serialize},
    Route,
};
use sea_orm::EntityTrait;
use sea_orm_rocket::Connection;
use serde_json::json;
use sha2::Sha256;

use crate::pool::Db;

#[derive(Serialize, Deserialize)]
#[serde(crate = "rocket::serde")]
pub struct RegisterData {
    username: String,
    password: String,
}

#[post("/register", data = "<register_data>")]
async fn index(
    conn: Connection<'_, Db>,
    register_data: Json<RegisterData>,
) -> Result<Json<Token>, (Status, String)> {
    let db = conn.into_inner();
    let register_data = register_data.into_inner();

    let argon2 = Argon2::default();

    let salt = SaltString::generate(&mut OsRng);
    let password_hash = argon2
        .hash_password(register_data.password.as_bytes(), &salt)
        .map_err(|e| (Status::BadRequest, e.to_string()))?
        .to_string();

    let new_user = ActiveUser {
        username: sea_orm::ActiveValue::Set(register_data.username),
        password: sea_orm::ActiveValue::Set(password_hash),
        created_at: sea_orm::ActiveValue::Set(Utc::now()),
        ..Default::default()
    };

    let user = UserTable::insert(new_user)
        .exec_with_returning(db)
        .await
        .map_err(|_| {
            (
                Status::Conflict,
                json!({ "username": "Already exists" }).to_string(),
            )
        })?;

    let key: Hmac<Sha256> = Hmac::new_from_slice(KEY_BYTES).unwrap();

    Ok(Json(Token {
        token: User {
            id: user.id,
            username: user.username,
            created_at: user.created_at,
        }
        .sign_with_key(&key)
        .map_err(|e| (Status::InternalServerError, e.to_string()))?,
    }))
}

pub fn routes() -> Vec<Route> {
    routes![index]
}
