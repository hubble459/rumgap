use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::FromQueryResult;

use crate::proto::ChapterReply;

#[derive(Debug, FromQueryResult)]
pub struct Full {
    pub id: i32,
    pub manga_id: i32,
    pub url: String,
    pub title: String,
    pub number: f32,
    pub posted: Option<DateTimeWithTimeZone>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,

    // special
    pub offset: Option<i32>,
}

impl Full {
   pub fn into_chapter_reply(self, index: i64) -> ChapterReply {
        ChapterReply {
            id: self.id,
            manga_id: self.manga_id,
            title: self.title,
            url: self.url,
            index,
            number: self.number,
            posted: self.posted.map(|date| date.timestamp_millis()),
            offset: self.offset,
            created_at: self.created_at.timestamp_millis(),
            updated_at: self.updated_at.timestamp_millis(),
        }
    }
}
