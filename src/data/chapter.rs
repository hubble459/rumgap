use sea_orm::prelude::DateTime;
use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::prelude::Decimal;
use sea_orm::FromQueryResult;

use crate::proto::{ChapterOffset, ChapterReply};

#[derive(Debug, FromQueryResult)]
pub struct Full {
    pub id: i32,
    pub manga_source_id: i32,
    pub url: String,
    pub title: String,
    pub number: f32,
    pub posted: Option<DateTimeWithTimeZone>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub canonical_chapter_id: Option<i32>,
    /// This chapter's canonical position - NULL only for a manually-unlinked chapter.
    /// Comparable across sources (unlike `index`, a purely per-source position) against
    /// `MangaReply.progress_ordinal` to correctly tell whether a chapter has been read.
    pub ordinal: Option<Decimal>,

    // special
    pub offset: Option<i32>,
    pub page: Option<i32>,
    pub fraction: Option<f32>,
}

impl Full {
    pub fn into_chapter_reply(self, index: i64) -> ChapterReply {
        ChapterReply {
            id: self.id,
            manga_source_id: self.manga_source_id,
            title: self.title,
            url: self.url,
            index,
            number: self.number,
            posted: self.posted.map(|date| date.timestamp_millis()),
            offset: self.offset.map(|offset| ChapterOffset {
                pixels: offset,
                page: self.page.unwrap(),
                fraction: self.fraction,
            }),
            canonical_chapter_id: self.canonical_chapter_id,
            ordinal: self.ordinal.and_then(|o| o.to_string().parse().ok()),
            created_at: self.created_at.and_utc().timestamp_millis(),
            updated_at: self.updated_at.and_utc().timestamp_millis(),
            missing: false,
            available_source_ids: vec![],
        }
    }
}

/// One canonical chapter of a manga as one source has it - or doesn't (`chapter` unset),
/// for `Chapter.Index` with `canonical`.
#[derive(Debug, FromQueryResult)]
pub struct Canonical {
    pub canonical_chapter_id: i32,
    pub ordinal: Decimal,
    /// This source's chapter linked to it, if any.
    pub id: Option<i32>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub number: Option<f32>,
    pub posted: Option<DateTimeWithTimeZone>,
    pub created_at: Option<DateTime>,
    pub updated_at: Option<DateTime>,
    /// The chapter's position among this source's own chapters, what `Chapter.Get` takes.
    pub source_index: Option<i64>,
    pub offset: Option<i32>,
    pub page: Option<i32>,
    pub fraction: Option<f32>,
    pub available_source_ids: Vec<i32>,
}

impl Canonical {
    pub fn into_chapter_reply(self, manga_source_id: i32) -> ChapterReply {
        let ordinal: Option<f64> = self.ordinal.to_string().parse().ok();
        let Some(id) = self.id else {
            return ChapterReply {
                manga_source_id,
                number: ordinal.unwrap_or_default() as f32,
                canonical_chapter_id: Some(self.canonical_chapter_id),
                ordinal,
                missing: true,
                available_source_ids: self.available_source_ids,
                ..Default::default()
            };
        };
        ChapterReply {
            id,
            manga_source_id,
            url: self.url.unwrap_or_default(),
            title: self.title.unwrap_or_default(),
            number: self.number.unwrap_or_default(),
            index: self.source_index.unwrap_or_default(),
            posted: self.posted.map(|date| date.timestamp_millis()),
            offset: self.offset.map(|offset| ChapterOffset {
                pixels: offset,
                page: self.page.unwrap_or_default(),
                fraction: self.fraction,
            }),
            created_at: self.created_at.map_or(0, |d| d.and_utc().timestamp_millis()),
            updated_at: self.updated_at.map_or(0, |d| d.and_utc().timestamp_millis()),
            canonical_chapter_id: Some(self.canonical_chapter_id),
            ordinal,
            missing: false,
            available_source_ids: vec![],
        }
    }
}
