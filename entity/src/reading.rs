use chrono::Utc;
use rocket::serde::{Deserialize, Serialize};
use sea_orm::{entity::prelude::*, ActiveValue};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Deserialize, Serialize)]
#[serde(crate = "rocket::serde")]
#[sea_orm(table_name = "reading")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: u32,
    pub user_id: u32,
    pub manga_id: String,
    pub read: u32,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "crate::user::Entity",
        from = "crate::reading::Column::UserId",
        to = "crate::user::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    User,
    #[sea_orm(
        belongs_to = "crate::manga::Entity",
        from = "crate::reading::Column::MangaId",
        to = "crate::manga::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    Manga,
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl ActiveModelBehavior for ActiveModel {
    fn new() -> Self {
        Self {
            created_at: ActiveValue::Set(Utc::now()),
            ..ActiveModelTrait::default()
        }
    }

    fn before_save(mut self, _insert: bool) -> Result<Self, DbErr> {
        self.updated_at = ActiveValue::Set(Utc::now());
        Ok(self)
    }
}
