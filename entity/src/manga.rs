//! `SeaORM` Entity. Generated by sea-orm-codegen 0.11.0

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "manga")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique)]
    pub url: String,
    pub title: String,
    pub description: String,
    pub cover: Option<String>,
    pub is_ongoing: bool,
    pub genres: Vec<String>,
    pub authors: Vec<String>,
    pub alt_titles: Vec<String>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::chapter::Entity")]
    Chapter,
}

impl Related<super::chapter::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Chapter.def()
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
