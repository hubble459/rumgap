use rocket::serde::{Deserialize, Serialize, Serializer};
use sea_orm::{entity::prelude::*, ActiveValue, IntoActiveModel};
use chrono::Utc;

pub const SPLITTER: &'static str = "{{||}}";

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Deserialize, Serialize)]
#[serde(crate = "rocket::serde")]
#[sea_orm(table_name = "manga")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: u32,
    #[sea_orm(unique)]
    pub url: String,
    pub title: String,
    #[sea_orm(column_type = "Text")]
    pub description: String,
    pub cover: Option<String>,
    pub ongoing: bool,
    #[sea_orm(column_type = "Text")]
    #[serde(serialize_with = "serialize_str_vec")]
    pub genres: String,
    #[sea_orm(column_type = "Text")]
    #[serde(serialize_with = "serialize_str_vec")]
    pub authors: String,
    #[sea_orm(column_type = "Text")]
    #[serde(serialize_with = "serialize_str_vec")]
    pub alt_titles: String,

    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::chapter::Entity")]
    Chapters,
}

impl Related<super::chapter::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Chapters.def()
    }
}

impl ActiveModelBehavior for ActiveModel {
    fn new() -> Self {
        Self {
            created_at: ActiveValue::Set(Utc::now()),
            updated_at: ActiveValue::Set(Utc::now()),
            ..ActiveModelTrait::default()
        }
    }

    fn before_save(mut self, _insert: bool) -> Result<Self, DbErr> {
        self.updated_at = ActiveValue::Set(Utc::now());
        Ok(self)
    }
}

use parser::model::Manga;

impl IntoActiveModel<ActiveModel> for Manga {
    fn into_active_model(self) -> ActiveModel {
        ActiveModel {
            id: ActiveValue::NotSet,
            url: ActiveValue::Set(self.url.to_string()),
            title: ActiveValue::Set(self.title),
            description: ActiveValue::Set(self.description),
            cover: ActiveValue::Set(self.cover.map(|c| c.to_string())),
            ongoing: ActiveValue::Set(self.ongoing),
            genres: ActiveValue::Set(self.genres.join(SPLITTER)),
            authors: ActiveValue::Set(self.authors.join(SPLITTER)),
            alt_titles: ActiveValue::Set(self.alt_titles.join(SPLITTER)),
            ..Default::default()
        }
    }
}

impl TryInto<Model> for ActiveModel {
    type Error = &'static str;

    fn try_into(self) -> Result<Model, Self::Error> {
        if self.id.is_not_set() {
            return Err("Id is not set");
        }

        Ok(Model {
            id: self.id.unwrap(),
            url: self.url.unwrap(),
            title: self.title.unwrap(),
            description: self.description.unwrap(),
            cover: self.cover.unwrap(),
            ongoing: self.ongoing.unwrap(),
            genres: self.genres.unwrap(),
            authors: self.authors.unwrap(),
            alt_titles: self.alt_titles.unwrap(),
            created_at: self.created_at.unwrap(),
            updated_at: self.updated_at.unwrap(),
        })
    }
}

fn serialize_str_vec<S>(str_list: &String, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let vec: Vec<&str> = str_list.split(SPLITTER).collect();

    vec.serialize(s)
}
