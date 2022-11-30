use sea_orm::{prelude::DateTimeWithTimeZone, FromQueryResult};
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Serialize, FromQueryResult)]
pub struct Partial {
    pub id: i32,
    pub permissions: i16,
    pub username: String,
    pub email: String,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Debug, Serialize, FromQueryResult)]
pub struct Full {
    pub id: i32,
    pub permissions: i16,
    pub username: String,
    pub email: String,
    pub count_following: i64,
    pub count_followers: i64,
    // pub count_reading: i64,
    // pub count_planned: i64,
    // pub count_completed: i64,
    // pub count_dropped: i64,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}