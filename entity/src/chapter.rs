//! `SeaORM` Entity

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "chapter")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub manga_source_id: i32,
    #[sea_orm(unique)]
    pub url: String,
    pub title: String,
    #[sea_orm(column_type = "Float")]
    pub number: f32,
    pub posted: Option<DateTimeWithTimeZone>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    /// NULL is reserved for the manual `UnlinkChapter` override case - every other
    /// chapter always ends up linked, even if only to a solo canonical row.
    pub canonical_chapter_id: Option<i32>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::chapter_offset::Entity")]
    ChapterOffset,
    #[sea_orm(
        belongs_to = "super::manga_source::Entity",
        from = "Column::MangaSourceId",
        to = "super::manga_source::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    MangaSource,
    #[sea_orm(
        belongs_to = "super::canonical_chapter::Entity",
        from = "Column::CanonicalChapterId",
        to = "super::canonical_chapter::Column::Id",
        on_update = "Cascade",
        on_delete = "SetNull"
    )]
    CanonicalChapter,
}

impl Related<super::chapter_offset::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ChapterOffset.def()
    }
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

impl Related<super::manga::Entity> for Entity {
    fn to() -> RelationDef {
        super::manga_source::Relation::Manga.def()
    }
    fn via() -> Option<RelationDef> {
        Some(Relation::MangaSource.def())
    }
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        super::chapter_offset::Relation::User.def()
    }
    fn via() -> Option<RelationDef> {
        Some(super::chapter_offset::Relation::Chapter.def().rev())
    }
}

impl ActiveModelBehavior for ActiveModel {}
