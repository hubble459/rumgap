use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use tonic::Status;

/// Encrypt a password with argon2
pub fn encrypt(password: &str) -> Result<String, Status> {
    let argon2 = Argon2::default();

    let salt = SaltString::generate(&mut OsRng);
    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| Status::aborted(e.to_string()))?
        .to_string();

    Ok(password_hash)
}

/// Verify a password hash with a plain password
pub fn verify(hash: &str, password: &str) -> Result<(), Status> {
    let argon2 = Argon2::default();

    let parsed_hash = PasswordHash::new(&hash).map_err(|e| Status::aborted(e.to_string()))?;

    argon2
        .verify_password(password.as_bytes(), &parsed_hash)
        .map_err(|_| Status::unauthenticated("Username and password mismatch"))?;

    Ok(())
}
