use actix_web::{Result, error::ErrorUnauthorized};

use crate::middleware::auth::{AuthService, UserPermissions};

pub fn can_edit(auth: AuthService, user_id: i32) -> Result<()> {
    let user = auth.user;

    if user.id != user_id && user.has_permission(UserPermissions::ADMIN) {
        Err(ErrorUnauthorized("Not allowed to edit this item"))
    } else {
        Ok(())
    }
}