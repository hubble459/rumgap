//! `SeaORM` Entity

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "manga_source")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub manga_id: i32,
    #[sea_orm(unique)]
    pub url: String,
    pub hostname: String,
    pub language: String,
    pub is_primary: bool,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub title: String,
    pub description: String,
    pub status: String,
    pub is_ongoing: bool,
    pub genres: Vec<String>,
    pub authors: Vec<String>,
    pub alt_titles: Vec<String>,
    /// When this source's details were last scraped; `None` if never since they moved here
    /// from `manga` (until then they're a copy of the primary source's).
    pub details_updated_at: Option<DateTime>,
    /// Raw scraped cover URL - not what's served to clients. `MangaSourceReply.cover` points
    /// at the locally-hosted copy instead, downloaded on first request (see
    /// src/util/cover_images.rs).
    pub cover_source_url: Option<String>,
    pub cover_status: String,
    pub cover_storage_key: Option<String>,
    pub cover_content_type: Option<String>,
    pub cover_checksum: Option<String>,
    pub cover_attempts: i32,
    /// The language the manga was first published in, when the site says (e.g. `ko`).
    pub original_language: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::chapter::Entity")]
    Chapter,
    #[sea_orm(
        belongs_to = "super::manga::Entity",
        from = "Column::MangaId",
        to = "super::manga::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Manga,
}

impl Related<super::chapter::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Chapter.def()
    }
}

impl Related<super::manga::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Manga.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
