use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::FromQueryResult;

#[derive(Debug, FromQueryResult)]
pub struct Full {
    pub id: i32,
    pub title: String,
    pub progress: i32,
    pub cover: Option<String>,
    pub count_chapters: i32,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}
