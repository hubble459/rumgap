use hmac::{Hmac, Mac};
use jwt::{SignWithKey, VerifyWithKey};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tonic::service::Interceptor;
use tonic::{Request, Status};

use crate::util::auth::Authorize;
use crate::util::db::DatabaseRequest;

lazy_static! {
    pub(crate) static ref SECRET_KEY: Hmac<Sha256> = Hmac::new_from_slice(
        std::env::var("JWT_SECRET")
            .expect("JWT_SECRET is not set")
            .as_bytes()
    )
    .unwrap();
}

/// JWT Token. Only valid while its `sid` session row exists, see `util::session`.
#[derive(Serialize, Deserialize)]
pub struct Token {
    pub id: i32,
    pub sid: String,
    pub iat: i64,
}

/// Sign JWT Token
pub fn sign(id: i32, session_id: &str) -> Result<String, jwt::Error> {
    Token {
        id,
        sid: session_id.to_string(),
        iat: chrono::Utc::now().timestamp(),
    }
    .sign_with_key(&SECRET_KEY.clone())
}

pub(crate) trait UserHasPermissions {
    /// Returns true if the user has permissions
    fn has_permission(&self, permission: UserPermissions) -> bool;
}

impl UserHasPermissions for entity::user::Model {
    fn has_permission(&self, permission: UserPermissions) -> bool {
        (UserPermissions::from_bits(self.permissions as u32).unwrap() & permission) == permission
    }
}

bitflags! {
    /// User Permission Bit Flags
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct UserPermissions: u32 {
        const USER = 0b00000001;
        const MOD = 0b00000010;
        const ADMIN = 0b00000100;
    }
}

/// Check if user is authenticated
pub async fn check_auth(mut req: Request<()>) -> Result<Request<()>, Status> {
    let db = req.db()?;
    let bearer_token = req.metadata().get("authorization");

    // Has authorization meta
    if let Some(bearer_token) = bearer_token {
        // Get value as string
        let bearer_token = bearer_token
            .to_str()
            .map_err(|_| Status::unauthenticated("Bearer token is invalid"))?;
        // Get token that's the suffix of "Bearer "
        let token = bearer_token.strip_prefix("Bearer ");
        // If there is a token
        if let Some(token) = token {
            // Verify it. A token that fails here (bad signature, pre-session format,
            // revoked or expired session) just leaves the request logged out, so calls
            // that don't need a user (like Login) still work with a stale header.
            let Ok(Token { id: user_id, sid, .. }) = token.verify_with_key(&SECRET_KEY.clone()) else {
                return Ok(req);
            };

            if let Some((session, user)) = crate::util::session::validate(db, user_id, &sid).await? {
                req.extensions_mut().insert(user);
                req.extensions_mut().insert(session);
            }
        } else {
            // "authorization" header was set but the token is invalid
            return Err(Status::unauthenticated("Bearer token is invalid"));
        }
    }

    Ok(req)
}

/// LoggedInCheck Struct to impl Interceptor for
#[derive(Clone)]
pub struct LoggedInCheck {
    perms: UserPermissions,
}

/// Implementation
impl LoggedInCheck {
    /// Create a new instance which checks for the specified perms
    pub fn new(perms: UserPermissions) -> Self {
        Self { perms }
    }
}

/// Implement tonic's Interceptor for LoggedInCheck
impl Interceptor for LoggedInCheck {
    /// When a request is made
    /// will check if user is logged in
    /// and has enough permissions to make the request
    ///
    /// If the user is not logged in or is missing permissions
    /// an error will be returned (Status)
    fn call(&mut self, req: tonic::Request<()>) -> Result<tonic::Request<()>, Status> {
        let user = req.authorize().ok();

        match user {
            Some(user) => {
                if user.has_permission(self.perms) {
                    Ok(req)
                } else {
                    Err(Status::permission_denied(
                        "You are missing permissions to make this call",
                    ))
                }
            }
            None => Err(Status::unauthenticated(
                "You need to be logged in to make this request!",
            )),
        }
    }
}
