//! `SeaORM` Entity

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "manga")]
/// Groups the sources of one manga. Its details (title, cover, ...) live on each
/// `manga_source`, since every site has its own.
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub nsfw: bool,
    /// manga, manhwa, manhua or unknown
    pub origin: String,
    pub origin_set_by_hand: bool,
    pub anilist_id: Option<i32>,
    /// Null when title matching never ran for it.
    pub anilist_match: Option<AnilistMatch>,
    pub anilist_matched_at: Option<DateTime>,
    pub anilist_title: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub anilist_description: Option<String>,
    pub anilist_cover: Option<String>,
    pub anilist_banner: Option<String>,
    pub anilist_score: Option<i32>,
    pub anilist_genres: Option<Vec<String>>,
    pub anilist_status: Option<String>,
    pub anilist_fetched_at: Option<DateTime>,
}

/// How a manga got its `anilist_id`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
pub enum AnilistMatch {
    /// Title matching found it.
    #[sea_orm(string_value = "auto")]
    Auto,
    /// A user picked it (or picked "not on AniList"); title matching leaves it alone.
    #[sea_orm(string_value = "manual")]
    Manual,
    /// Title matching found nothing.
    #[sea_orm(string_value = "none")]
    None,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::manga_source::Entity")]
    MangaSource,
    #[sea_orm(has_many = "super::canonical_chapter::Entity")]
    CanonicalChapter,
    #[sea_orm(has_many = "super::reading::Entity")]
    Reading,
}

impl Related<super::manga_source::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::MangaSource.def()
    }
}

impl Related<super::canonical_chapter::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::CanonicalChapter.def()
    }
}

impl Related<super::reading::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Reading.def()
    }
}

/// A manga no longer `belongs_to` chapter directly (it's now via manga_source), so this
/// is a chained (`via`) relation rather than a direct FK-based one - not usable with raw
/// `.join(JoinType::_, Relation::Chapter.def())` query-builder calls, which now need to
/// hop through `manga_source` explicitly instead.
impl Related<super::chapter::Entity> for Entity {
    fn to() -> RelationDef {
        super::manga_source::Relation::Chapter.def()
    }
    fn via() -> Option<RelationDef> {
        Some(Relation::MangaSource.def())
    }
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        super::reading::Relation::User.def()
    }
    fn via() -> Option<RelationDef> {
        Some(super::reading::Relation::Manga.def().rev())
    }
}

impl ActiveModelBehavior for ActiveModel {}
