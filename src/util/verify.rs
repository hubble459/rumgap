use migration::DbErr;
use regex::Regex;
use tonic::Status;

lazy_static! {
    static ref ONLY_WORD: Regex = Regex::new(r"^\w+$").unwrap();
    static ref HAS_UPPERCASE: Regex = Regex::new("[A-Z]").unwrap();
    static ref HAS_DIGIT: Regex = Regex::new("[0-9]").unwrap();
    static ref HAS_SPECIAL: Regex = Regex::new(r"\W").unwrap();
    static ref IS_EMAIL: Regex = Regex::new(r#"(?:[a-z0-9!#$%&'*+/=?^_`{|}~-]+(?:\.[a-z0-9!#$%&'*+/=?^_`{|}~-]+)*|"(?:[\x01-\x08\x0b\x0c\x0e-\x1f\x21\x23-\x5b\x5d-\x7f]|\\[\x01-\x09\x0b\x0c\x0e-\x7f])*")@(?:(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+[a-z0-9](?:[a-z0-9-]*[a-z0-9])?|\[(?:(?:(2(5[0-5]|[0-4][0-9])|1[0-9][0-9]|[1-9]?[0-9]))\.){3}(?:(2(5[0-5]|[0-4][0-9])|1[0-9][0-9]|[1-9]?[0-9])|[a-z0-9-]*[a-z0-9]:(?:[\x01-\x08\x0b\x0c\x0e-\x1f\x21-\x5a\x53-\x7f]|\\[\x01-\x09\x0b\x0c\x0e-\x7f])+)\])"#).unwrap();
}

/// Verify that username:
/// - is at least 4 characters
/// - contains only normal characters
/// - is lowercased
pub fn username(username: &str) -> Result<String, Status> {
    let username = username.trim();

    if username.len() < 4 {
        Err(Status::invalid_argument(
            "Username should be at least 4 characters",
        ))
    } else if !ONLY_WORD.is_match(username) {
        Err(Status::invalid_argument(
            "Username should only contain [a-zA-Z0-9_]+",
        ))
    } else {
        Ok(username.to_ascii_lowercase())
    }
}

/// Verify that email:
/// - is valid
/// - is lowercase
pub fn email(email: &str) -> Result<String, Status> {
    let email = email.trim();

    if !IS_EMAIL.is_match(email) {
        Err(Status::invalid_argument("Invalid email"))
    } else {
        Ok(email.to_ascii_lowercase())
    }
}

/// Verify that password:
/// - contains a digit
/// - contains a special character
/// - contains an uppercase
/// - is at least 8 chars
pub fn password(email: &str) -> Result<String, Status> {
    let password = email.trim();
    let mut errors = vec![];

    if !HAS_DIGIT.is_match(password) {
        errors.push("Missing digit");
    }
    if !HAS_SPECIAL.is_match(password) {
        errors.push("Missing special character");
    }
    if !HAS_UPPERCASE.is_match(password) {
        errors.push("Missing uppercase");
    }
    if password.len() < 8 {
        errors.push("Should be more than 8 characters");
    }

    if !errors.is_empty() {
        Err(Status::invalid_argument(errors.join("\n")))
    } else {
        Ok(String::from(password))
    }
}

/// Verify DB Error is a Conflict Error
pub fn is_conflict(err: &DbErr) -> bool {
    if let DbErr::Query(sea_orm::RuntimeErr::SqlxError(e)) = err {
        if let Some(db_err) = e.as_database_error() {
            if db_err.code() == Some(std::borrow::Cow::Borrowed("23505")) {
                return true;
            }
        }
    }
    false
}
