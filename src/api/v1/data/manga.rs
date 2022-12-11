use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::{DeriveColumn, EnumIter, FromQueryResult, IdenStatic};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct Post {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct Patch {
    pub url: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveColumn)]
pub enum Minimal {
    Url,
    UpdatedAt,
}

#[derive(Debug, Serialize, FromQueryResult)]
pub struct Full {
    pub id: i32,
    pub url: String,
    pub title: String,
    pub description: String,
    pub cover: Option<String>,
    pub is_ongoing: bool,
    pub genres: Vec<String>,
    pub authors: Vec<String>,
    pub alt_titles: Vec<String>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
    // special
    pub count_chapters: i64,
    pub next_update: Option<DateTimeWithTimeZone>,
    pub last_updated: DateTimeWithTimeZone,
}
