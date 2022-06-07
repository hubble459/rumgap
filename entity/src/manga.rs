use rocket::serde::{Deserialize, Serialize};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Deserialize, Serialize)]
#[serde(crate = "rocket::serde")]
#[sea_orm(table_name = "manga")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique)]
    pub url: String,
    pub title: String,
    #[sea_orm(column_type = "Text")]
    pub description: String,
    pub cover: Option<String>,
    pub ongoing: bool,
    #[sea_orm(column_type = "Text")]
    pub genres: String,
    #[sea_orm(column_type = "Text")]
    pub authors: String,
    #[sea_orm(column_type = "Text")]
    pub alt_titles: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::chapter::Entity")]
    Chapters,
}

impl ActiveModelBehavior for ActiveModel {}
