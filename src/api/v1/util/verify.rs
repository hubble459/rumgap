use actix_web::{error::ErrorBadRequest, Result};
use regex::Regex;

lazy_static! {
    static ref ONLY_WORD: Regex = Regex::new(r"^\w+$").unwrap();
    static ref HAS_UPPERCASE: Regex = Regex::new("[A-Z]").unwrap();
    static ref HAS_DIGIT: Regex = Regex::new("[0-9]").unwrap();
    static ref HAS_SPECIAL: Regex = Regex::new(r"\W").unwrap();
    static ref IS_EMAIL: Regex = Regex::new(r#"(?:[a-z0-9!#$%&'*+/=?^_`{|}~-]+(?:\.[a-z0-9!#$%&'*+/=?^_`{|}~-]+)*|"(?:[\x01-\x08\x0b\x0c\x0e-\x1f\x21\x23-\x5b\x5d-\x7f]|\\[\x01-\x09\x0b\x0c\x0e-\x7f])*")@(?:(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+[a-z0-9](?:[a-z0-9-]*[a-z0-9])?|\[(?:(?:(2(5[0-5]|[0-4][0-9])|1[0-9][0-9]|[1-9]?[0-9]))\.){3}(?:(2(5[0-5]|[0-4][0-9])|1[0-9][0-9]|[1-9]?[0-9])|[a-z0-9-]*[a-z0-9]:(?:[\x01-\x08\x0b\x0c\x0e-\x1f\x21-\x5a\x53-\x7f]|\\[\x01-\x09\x0b\x0c\x0e-\x7f])+)\])"#).unwrap();
}

pub fn username(username: &str) -> Result<String> {
    let username = username.trim();

    if username.len() < 4 {
        Err(ErrorBadRequest(
            "Username should be at least 4 characters",
        ))
    } else if !ONLY_WORD.is_match(username) {
        Err(ErrorBadRequest(
            "Username should only contain [a-zA-Z0-9_]+",
        ))
    } else {
        Ok(String::from(username))
    }
}

pub fn email(email: &str) -> Result<String> {
    let email = email.trim();

    if !IS_EMAIL.is_match(email) {
        Err(ErrorBadRequest("Invalid email"))
    } else {
        Ok(String::from(email))
    }
}

pub fn password(email: &str) -> Result<String> {
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
        Err(ErrorBadRequest(errors.join("\n")))
    } else {
        Ok(String::from(password))
    }
}
