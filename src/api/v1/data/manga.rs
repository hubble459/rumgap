use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::{DeriveColumn, EnumIter, FromQueryResult, IdenStatic};
use serde::{Deserialize, Serialize};

use super::paginate::PaginateQuery;

#[derive(Debug, Deserialize)]
pub struct Post {
    pub urls: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Patch {
    pub url: String,
}

#[derive(Debug, Copy, Clone, EnumIter, DeriveColumn)]
pub enum Minimal {
    Url,
    UpdatedAt,
}

#[derive(Debug, Deserialize)]
pub struct IndexQuery {
    #[serde(flatten)]
    pub paginate: PaginateQuery,
    pub search: Option<String>,
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
    pub next: Option<DateTimeWithTimeZone>,
    pub last: Option<DateTimeWithTimeZone>,
}
