use chrono::Utc;
use rocket::serde::{Deserialize, Serialize};
use sea_orm::entity::prelude::*;
use sea_orm::ActiveValue;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Deserialize, Serialize)]
#[serde(crate = "rocket::serde")]
#[sea_orm(table_name = "user")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: u32,
    #[sea_orm(unique)]
    pub username: String,
    #[serde(skip)]
    pub password: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "crate::reading::Entity")]
    Reading,
}

impl ActiveModelBehavior for ActiveModel {
    fn new() -> Self {
        Self {
            created_at: sea_orm::ActiveValue::Set(Utc::now()),
            ..ActiveModelTrait::default()
        }
    }

    fn before_save(mut self, _insert: bool) -> Result<Self, DbErr> {
        self.updated_at = ActiveValue::Set(Utc::now());
        Ok(self)
    }
}
