//! `SeaORM` Entity for the per-scrape-attempt audit trail backing the admin-only
//! `Scraper.Status` RPC (see `service::v1::scraper` and `util::scrape_log` in the main crate).
//!
//! Deliberately has no relations: `manga_id`/`manga_source_id` are plain, unenforced
//! references rather than foreign keys, so a row's scrape history survives the manga/source
//! itself being deleted.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "scrape_log")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub hostname: String,
    /// `manga | chapter_images` -- which manga_parser call this attempt was.
    pub operation: String,
    pub url: String,
    pub manga_id: Option<i32>,
    pub manga_source_id: Option<i32>,
    pub success: bool,
    /// `ScrapeError` variant name (e.g. `ReqwestError`), NULL on success.
    pub error_type: Option<String>,
    pub error_message: Option<String>,
    pub duration_ms: i32,
    pub created_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
