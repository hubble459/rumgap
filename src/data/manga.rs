use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::{DeriveColumn, EnumIter, FromQueryResult};

use crate::proto::MangaReply;

#[derive(Debug, Copy, Clone, EnumIter, DeriveColumn)]
pub enum Minimal {
    Url,
    UpdatedAt,
}

#[derive(Debug, FromQueryResult)]
pub struct Full {
    pub id: i32,
    pub url: String,
    pub title: String,
    pub description: String,
    pub cover: Option<String>,
    pub is_ongoing: bool,
    pub progress: Option<i32>,
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
            reading_progress: value.progress,
            last: value.last.map(|date| date.timestamp_millis()),
            next: value.next.map(|date| date.timestamp_millis()),
            created_at: value.created_at.timestamp_millis(),
            updated_at: value.updated_at.timestamp_millis(),
        }
    }
}
