use parser::parser::{MangaParser, Parser};
use parser::Url;
use rocket::http::Status;
use rocket::response::content::RawJson;
use rocket::serde::json::Json;
use rocket::serde::{Deserialize, Serialize};
use rocket::{Route, State};
use sea_orm::{entity::*, query::*};
use sea_orm_rocket::Connection;

use entity::chapter;
use entity::chapter::Entity as Chapter;
use entity::manga::Entity as Manga;
use entity::manga::{self, SPLITTER};
use serde_json::json;

use crate::pagination::Pagination;
use crate::pool::Db;

pub const DEFAULT_LIMIT: usize = 10;

pub fn map_manga_json(mut manga: JsonValue) -> JsonValue {
    manga["alt_titles"] = manga["alt_titles"]
        .as_str()
        .unwrap()
        .split(SPLITTER)
        .collect();
    manga["genres"] = manga["genres"].as_str().unwrap().split(SPLITTER).collect();
    manga["authors"] = manga["authors"].as_str().unwrap().split(SPLITTER).collect();
    manga
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "rocket::serde")]
struct MangaUrl {
    url: String,
}

#[post("/", data = "<manga_url>")]
async fn create(
    conn: Connection<'_, Db>,
    manga_url: Json<MangaUrl>,
    parser: &State<MangaParser>,
) -> Result<Json<manga::Model>, Status> {
    let db = conn.into_inner();

    let url = manga_url.into_inner().url;

    let manga = parser
        .manga(Url::parse(&url).map_err(|_| Status::BadRequest)?)
        .await
        .map_err(|_| Status::InternalServerError)?;

    let stored = manga
        .clone()
        .into_active_model()
        .save(db)
        .await
        .map_err(|_| Status::InternalServerError)?;

    // Store Chapters
    Chapter::insert_many(manga.chapters.iter().map(|chapter| chapter::ActiveModel {
        manga_id: stored.id.clone(),
        url: ActiveValue::Set(chapter.url.to_string()),
        title: ActiveValue::Set(chapter.title.to_owned()),
        number: ActiveValue::Set(chapter.number),
        posted: ActiveValue::Set(chapter.posted),
        ..Default::default()
    }))
    .exec(db)
    .await
    .map_err(|_| Status::InternalServerError)?;

    Ok(Json(
        stored.try_into().map_err(|_| Status::InternalServerError)?,
    ))
}

#[get("/?<page>&<limit>")]
async fn list(
    conn: Connection<'_, Db>,
    page: Option<usize>,
    limit: Option<usize>,
) -> Result<Json<Pagination<Vec<JsonValue>>>, (Status, RawJson<JsonValue>)> {
    let db = conn.into_inner();

    // Set page number and items per page
    let page = page.unwrap_or(1);
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if page == 0 {
        return Err((
            Status::BadRequest,
            RawJson(json!({"message": "page should be bigger than 0"})),
        ));
    } else if limit == 0 {
        return Err((
            Status::BadRequest,
            RawJson(json!({"message": "limit should be bigger than 0"})),
        ));
    }

    // Setup paginator
    let paginator = Manga::find()
        .order_by_asc(manga::Column::Id)
        .join_rev(JoinType::InnerJoin, chapter::Relation::Manga.def())
        .column_as(chapter::Column::MangaId.count(), "chapter_count")
        .into_json()
        .paginate(db, limit);
    let num_pages = paginator.num_pages().await.map_err(|e| {
        (
            Status::BadRequest,
            RawJson(json!({"message": e.to_string()})),
        )
    })?;

    // Fetch paginated manga
    let manga = paginator.fetch_page(page - 1).await.map_err(|e| {
        (
            Status::BadRequest,
            RawJson(json!({"message": e.to_string()})),
        )
    })?;

    Ok(Json(Pagination {
        page,
        limit,
        num_pages,
        data: manga.into_iter().map(map_manga_json).collect(),
    }))
}

#[get("/<id>")]
async fn get(conn: Connection<'_, Db>, id: u32) -> Option<Json<JsonValue>> {
    let db = conn.into_inner();

    let manga = Manga::find_by_id(id)
        .join_rev(JoinType::InnerJoin, chapter::Relation::Manga.def())
        .column_as(chapter::Column::MangaId.count(), "chapter_count")
        .into_json()
        .one(db)
        .await
        .unwrap();

    manga.map(|manga| Json(map_manga_json(manga)))
}

#[delete("/<id>")]
async fn delete(conn: Connection<'_, Db>, id: u32) -> &'static str {
    let db = conn.into_inner();

    let manga: manga::ActiveModel = Manga::find_by_id(id).one(db).await.unwrap().unwrap().into();

    manga.delete(db).await.unwrap();

    "Manga successfully deleted"
}

pub fn routes() -> Vec<Route> {
    routes![create, delete, list, get]
}

pub fn base() -> &'static str {
    "manga"
}
