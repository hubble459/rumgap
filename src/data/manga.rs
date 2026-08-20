use sea_orm::prelude::{DateTime, DateTimeWithTimeZone};
use sea_orm::{DeriveColumn, EnumIter, FromQueryResult};

use crate::proto::{MangaReply, MangaSourceReply};

#[derive(Debug, Copy, Clone, EnumIter, DeriveColumn)]
pub enum Minimal {
    UpdatedAt,
}

#[derive(Debug, FromQueryResult)]
pub struct Full {
    pub id: i32,
    pub title: String,
    pub description: String,
    pub cover: Option<String>,
    pub status: String,
    pub is_ongoing: bool,
    pub progress: Option<i32>,
    pub genres: Vec<String>,
    pub authors: Vec<String>,
    pub alt_titles: Vec<String>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    // special
    // NOTE: count_chapters/last/next are computed from the primary source's chapters only.
    pub count_chapters: i64,
    pub next: Option<DateTimeWithTimeZone>,
    pub last: Option<DateTimeWithTimeZone>,
}

impl Full {
    /// `sources` has to be fetched (and, for `Manga.Index`/`Manga.Similar`, batched) separately
    /// since it's a one-to-many list, not something that fits this struct's flat-row shape.
    pub fn into_manga_reply(self, sources: Vec<MangaSourceReply>) -> MangaReply {
        MangaReply {
            id: self.id,
            title: self.title,
            description: self.description,
            cover: self.cover,
            status: self.status,
            is_ongoing: self.is_ongoing,
            genres: self.genres,
            authors: self.authors,
            alt_titles: self.alt_titles,
            count_chapters: self.count_chapters,
            reading_progress: self.progress,
            last: self.last.map(|date| date.timestamp_millis()),
            next: self.next.map(|date| date.timestamp_millis()),
            created_at: self.created_at.and_utc().timestamp_millis(),
            updated_at: self.updated_at.and_utc().timestamp_millis(),
            sources,
        }
    }
}

impl From<entity::manga_source::Model> for MangaSourceReply {
    fn from(value: entity::manga_source::Model) -> Self {
        Self {
            id: value.id,
            url: value.url,
            hostname: value.hostname,
            language: value.language,
            is_primary: value.is_primary,
            created_at: value.created_at.and_utc().timestamp_millis(),
            updated_at: value.updated_at.and_utc().timestamp_millis(),
        }
    }
}
