use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::FromQueryResult;
use serde::{Deserialize, Serialize};

use super::paginate::PaginateQuery;

#[derive(Debug, Deserialize)]
pub struct Post {
    pub manga_id: i32,
}

#[derive(Debug, Deserialize)]
pub struct Patch {
    pub progress: i32,
}

#[derive(Debug, Deserialize)]
pub struct IndexQuery {
    #[serde(flatten)]
    pub paginate: PaginateQuery,
    pub search: Option<String>,
    pub order: Option<String>,
}

#[derive(Debug, Serialize, FromQueryResult)]
pub struct Full {
    pub id: i32,
    pub title: i32,
    pub progress: i32,
    pub cover: Option<String>,
    pub count_chapters: i32,

    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}
