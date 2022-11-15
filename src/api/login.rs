use crate::auth::{User, KEY_BYTES};
use argon2::{
    password_hash::{PasswordHash, PasswordVerifier},
    Argon2,
};
use entity::user;
use entity::user::Entity as UserTable;
use hmac::{Hmac, Mac};
use jwt::SignWithKey;
use rocket::serde::json::Json;
use rocket::{http::Status, response::content::RawJson};
use rocket::{
    serde::{Deserialize, Serialize},
    Route,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use sea_orm_rocket::Connection;
use serde_json::json;
use sha2::Sha256;

use crate::pool::Db;

#[derive(Serialize, Deserialize)]
#[serde(crate = "rocket::serde")]
pub struct LoginData {
    username: String,
    password: String,
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "rocket::serde")]
pub struct  Token {
    pub token: String,
}

#[post("/login", data = "<login_data>")]
async fn index(
    conn: Connection<'_, Db>,
    login_data: Json<LoginData>,
) -> Result<Json<Token>, (Status, RawJson<String>)> {
    let db = conn.into_inner();
    let login_data = login_data.into_inner();

    let user = UserTable::find()
        .filter(user::Column::Username.eq(login_data.username))
        .one(db)
        .await
        .map_err(|e| (Status::InternalServerError, RawJson(e.to_string())))?
        .ok_or((
            Status::Unauthorized,
            RawJson(json!({ "username": "Not found" }).to_string()),
        ))?;

    let argon2 = Argon2::default();

    let parsed_hash = PasswordHash::new(&user.password)
        .map_err(|e| (Status::InternalServerError, RawJson(e.to_string())))?;

    argon2
        .verify_password(login_data.password.as_bytes(), &parsed_hash)
        .map_err(|_| {
            (
                Status::Unauthorized,
                RawJson(json!({ "password": "Does not match" }).to_string()),
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
        .map_err(|e| (Status::InternalServerError, RawJson(e.to_string())))?,
    }))
}

pub fn routes() -> Vec<Route> {
    routes![index]
}
