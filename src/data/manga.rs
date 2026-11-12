use sea_orm::prelude::{DateTime, DateTimeWithTimeZone, Decimal};
use sea_orm::FromQueryResult;

use crate::proto::{MangaReply, MangaSourceReply};

/// A manga as one user sees it: its details come from the source they see it through
/// (`selected_source_id`, see `DISPLAY_SOURCE_QUERY` in `service::v1::manga`).
#[derive(Debug, FromQueryResult)]
pub struct Full {
    pub id: i32,
    pub selected_source_id: Option<i32>,
    pub title: Option<String>,
    pub description: Option<String>,
    /// The selected source when it has a cover, else the primary when that has one.
    pub cover_source_id: Option<i32>,
    pub status: Option<String>,
    pub is_ongoing: Option<bool>,
    pub progress: Option<i32>,
    /// Canonical ordinal of `progress`'s underlying `last_canonical_chapter_id` - comparable
    /// across sources against `ChapterReply.ordinal`, unlike `progress` itself (a rank among
    /// canonical slots, which isn't the same scale as any one source's own chapter count).
    pub progress_ordinal: Option<Decimal>,
    pub genres: Option<Vec<String>>,
    pub authors: Option<Vec<String>>,
    pub alt_titles: Option<Vec<String>>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    // special
    // NOTE: count_chapters/last/next are computed from the selected source's chapters only.
    pub count_chapters: i64,
    pub next: Option<DateTimeWithTimeZone>,
    pub last: Option<DateTimeWithTimeZone>,
}

/// Where a source's locally-hosted cover is served.
pub fn source_cover_url(source_id: i32) -> String {
    crate::util::signed_url::path_url(&format!("/covers/source/{source_id}"))
}

impl Full {
    /// `sources` has to be fetched (and, for `Manga.Index`/`Manga.Similar`, batched) separately
    /// since it's a one-to-many list, not something that fits this struct's flat-row shape.
    pub fn into_manga_reply(self, sources: Vec<MangaSourceReply>) -> MangaReply {
        MangaReply {
            id: self.id,
            title: self.title.unwrap_or_default(),
            description: self.description.unwrap_or_default(),
            // Never blocks on a download (same lazy pattern as chapter images) - the actual
            // download happens on first real request to this URL, in the image server.
            cover: self.cover_source_id.map(source_cover_url),
            status: self.status.unwrap_or_default(),
            is_ongoing: self.is_ongoing.unwrap_or_default(),
            genres: self.genres.unwrap_or_default(),
            authors: self.authors.unwrap_or_default(),
            alt_titles: self.alt_titles.unwrap_or_default(),
            count_chapters: self.count_chapters,
            reading_progress: self.progress,
            progress_ordinal: self.progress_ordinal.and_then(|o| o.to_string().parse().ok()),
            last: self.last.map(|date| date.timestamp_millis()),
            next: self.next.map(|date| date.timestamp_millis()),
            created_at: self.created_at.and_utc().timestamp_millis(),
            updated_at: self.updated_at.and_utc().timestamp_millis(),
            selected_source_id: self.selected_source_id.unwrap_or_default(),
            sources,
        }
    }
}

/// `count_chapters`, `last` and `fully_stored` are filled in by `load_manga_sources`.
impl From<entity::manga_source::Model> for MangaSourceReply {
    fn from(value: entity::manga_source::Model) -> Self {
        Self {
            cover: value.cover_source_url.as_ref().map(|_| source_cover_url(value.id)),
            id: value.id,
            url: value.url,
            hostname: value.hostname,
            language: value.language,
            is_primary: value.is_primary,
            created_at: value.created_at.and_utc().timestamp_millis(),
            updated_at: value.updated_at.and_utc().timestamp_millis(),
            fully_stored: false,
            title: value.title,
            description: value.description,
            genres: value.genres,
            authors: value.authors,
            alt_titles: value.alt_titles,
            status: value.status,
            is_ongoing: value.is_ongoing,
            count_chapters: 0,
            last: None,
            details_updated_at: value.details_updated_at.map(|date| date.and_utc().timestamp_millis()),
        }
    }
}
