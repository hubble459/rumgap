use hmac::{Hmac, Mac};
use jwt::VerifyWithKey;
use rocket::{
    http::Status,
    request::{self, FromRequest, Request},
    serde::{Deserialize, Serialize},
};
use sea_orm::prelude::DateTimeUtc;
use sha2::Sha256;

#[derive(Debug, Deserialize, Serialize)]
#[serde(crate = "rocket::serde")]
pub struct User {
    pub id: u32,
    pub username: String,
    pub created_at: DateTimeUtc,
}

#[derive(Debug)]
pub enum UserError {
    TokenError(jwt::Error),
    MissingToken,
}

pub const KEY_BYTES: &[u8; 14] = b"*moans cutely*";

#[rocket::async_trait]
impl<'r> FromRequest<'r> for User {
    type Error = UserError;

    async fn from_request(req: &'r Request<'_>) -> request::Outcome<Self, Self::Error> {
        let bearer = req.headers().get_one("Authorization");

        if bearer.is_none() {
            return request::Outcome::Failure((Status::Unauthorized, UserError::MissingToken));
        }
        let bearer = bearer.unwrap().split_once(" ").unwrap().1;

        let key: Hmac<Sha256> = Hmac::new_from_slice(KEY_BYTES).unwrap();
        let result: Result<User, jwt::Error> = bearer.verify_with_key(&key);

        if let Ok(user) = result {
            return request::Outcome::Success(user);
        } else {
            return request::Outcome::Failure((
                Status::Unauthorized,
                UserError::TokenError(result.err().unwrap()),
            ));
        }
    }
}
