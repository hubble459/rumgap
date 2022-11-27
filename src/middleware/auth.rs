use hmac::{Hmac, Mac};
use jwt::{SignWithKey, VerifyWithKey};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

use actix_web::{error::ErrorUnauthorized, Error, FromRequest};
use sha2::Sha256;

#[derive(Serialize, Deserialize)]
pub struct Token {
    pub id: u32,
}

lazy_static! {
    static ref SECRET_KEY: Hmac<Sha256> = Hmac::new_from_slice(b"bUHhhHH#!bU@NkNUnK12").unwrap();
}

fn sign(id: u32) -> Result<String, jwt::Error> {
    Token { id }.sign_with_key(&SECRET_KEY.clone())
}

pub struct AuthService {
    user: u32,
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

                Ok(Self { user: id })
            } else {
                Err(ErrorUnauthorized("Missing Bearer token"))
            }
        })
    }

    fn extract(req: &actix_web::HttpRequest) -> Self::Future {
        Self::from_request(req, &mut actix_web::dev::Payload::None)
    }
}
