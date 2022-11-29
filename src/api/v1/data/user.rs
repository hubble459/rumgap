use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Post {
    pub username: String,
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct Patch {
    pub username: Option<String>,
    pub email: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Login {
    pub username: Option<String>,
    pub email: Option<String>,
    pub password: String,
}
