use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::{DeriveColumn, EnumIter, FromQueryResult, IdenStatic};
use serde::{Deserialize, Serialize};

use crate::proto::MangaReply;

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

impl From<Full> for MangaReply {
    fn from(value: Full) -> Self {
        Self {
            id: value.id,
            url: value.url,
            title: value.title,
            description: value.description,
            cover: value.cover,
            is_ongoing: value.is_ongoing,
            genres: value.genres,
            authors: value.authors,
            alt_titles: value.alt_titles,
            count_chapters: value.count_chapters,
            last: value.last.map(|date| date.timestamp_millis()),
            next: value.next.map(|date| date.timestamp_millis()),
            created_at: value.created_at.timestamp_millis(),
            updated_at: value.updated_at.timestamp_millis(),
        }
    }
}
