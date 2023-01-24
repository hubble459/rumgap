//! `SeaORM` Entity. Generated by sea-orm-codegen 0.10.7

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "chapter")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub manga_id: i32,
    #[sea_orm(unique)]
    pub url: String,
    pub title: String,
    pub number: f32,
    pub posted: Option<DateTimeWithTimeZone>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::manga::Entity",
        from = "Column::MangaId",
        to = "super::manga::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Manga,
}

impl Related<super::manga::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Manga.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
