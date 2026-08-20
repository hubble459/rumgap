//! `SeaORM` Entity

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "manga")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub title: String,
    pub description: String,
    pub cover: Option<String>,
    pub is_ongoing: bool,
    pub genres: Vec<String>,
    pub authors: Vec<String>,
    pub alt_titles: Vec<String>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub status: String,
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
