//! `SeaORM` Entity for the local image-hosting cache.
//!
//! Keyed by `(chapter_id, page_index)` -- no UUIDs, `chapter_id` is already a
//! unique integer PK and `page_index` a plain ordinal, which is already a
//! sufficient, human-debuggable key.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "chapter_image")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub chapter_id: i32,
    #[sea_orm(primary_key, auto_increment = false)]
    pub page_index: i32,
    /// `pending | downloading | done | failed` -- plain string, matching the
    /// existing `manga.status` convention rather than a Postgres enum type.
    pub status: String,
    pub source_url: String,
    pub storage_key: Option<String>,
    pub content_type: Option<String>,
    pub byte_size: Option<i64>,
    pub checksum: Option<String>,
    pub error: Option<String>,
    pub attempts: i32,
    /// Read straight from the header bytes at download time (no decoding needed) - NULL if
    /// the download failed, or the format isn't PNG/JPEG. Lets the reader size each page's
    /// placeholder correctly before the image loads, avoiding a scroll-jumping relayout.
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::chapter::Entity",
        from = "Column::ChapterId",
        to = "super::chapter::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Chapter,
}

impl Related<super::chapter::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Chapter.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
