use rocket::serde::{Deserialize, Serialize};
use sea_orm::entity::prelude::*;
use chrono::Utc;
use sea_orm::ActiveValue;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Deserialize, Serialize)]
#[serde(crate = "rocket::serde")]
#[sea_orm(table_name = "chapter")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: u32,
    pub manga_id: u32,
    #[sea_orm(unique)]
    pub url: String,
    pub title: String,
    pub number: f32,
    pub posted: Option<DateTimeUtc>,

    pub created_at: DateTimeUtc,
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

impl ActiveModelBehavior for ActiveModel {
    fn new() -> Self {
        Self {
            created_at: ActiveValue::Set(Utc::now()),
            ..ActiveModelTrait::default()
        }
    }
}
