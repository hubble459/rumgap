//! `SeaORM` Entity. Generated by sea-orm-codegen 0.11.0

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "user")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub permissions: i16,
    #[sea_orm(unique)]
    pub username: String,
    #[sea_orm(unique)]
    pub email: String,
    pub password_hash: String,
    pub preferred_hostnames: Vec<String>,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl Related<super::manga::Entity> for Entity {
    fn to() -> RelationDef {
        super::reading::Relation::Manga.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::reading::Relation::User.def().rev())
    }
}

impl Related<super::chapter::Entity> for Entity {
    fn to() -> RelationDef {
        super::chapter_offset::Relation::Chapter.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::chapter_offset::Relation::User.def().rev())
    }
}

impl ActiveModelBehavior for ActiveModel {}
