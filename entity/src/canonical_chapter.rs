//! `SeaORM` Entity

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "canonical_chapter")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub manga_id: i32,
    /// NUMERIC(10,3), not float: ordinal equality is load-bearing for the
    /// canonical-chapter matching heuristic and float equality is a classic footgun.
    pub ordinal: Decimal,
    pub created_at: DateTime,
    pub updated_at: DateTime,
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
