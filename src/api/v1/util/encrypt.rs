use actix_web::error::{ErrorInternalServerError, ErrorServiceUnavailable, ErrorUnauthorized};
use actix_web::Result;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

pub fn encrypt(password: &str) -> Result<String> {
    let argon2 = Argon2::default();

    let salt = SaltString::generate(&mut OsRng);
    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(ErrorServiceUnavailable)?
        .to_string();

    Ok(password_hash)
}

pub fn verify(hash: &str, password: &str) -> Result<()> {
    let argon2 = Argon2::default();

    let parsed_hash = PasswordHash::new(&hash).map_err(ErrorInternalServerError)?;

    argon2
        .verify_password(password.as_bytes(), &parsed_hash)
        .map_err(|_| ErrorUnauthorized("Username and password mismatch"))?;

    Ok(())
}
