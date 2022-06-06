use parser::parser::{MangaParser, Parser};
use parser::Url;
use rocket::http::Status;
use rocket::response::content::RawJson;
use rocket::serde::json::Json;
use rocket::serde::{Deserialize, Serialize};
use rocket::{Route, State};
use sea_orm::{entity::*, query::*};
use sea_orm_rocket::Connection;
use serde_json::json;

use entity::manga;
use entity::manga::Entity as Manga;

use crate::pool::Db;

const DEFAULT_LIMIT: usize = 5;

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

    let stored = manga::ActiveModel {
        title: Set(manga.title),
        description: Set(manga.description),
        ..Default::default()
    }
    .save(db)
    .await
    .map_err(|_| Status::InternalServerError)?;

    Ok(Json(manga::Model {
        id: stored.id.unwrap(),
        title: stored.title.unwrap(),
        description: stored.description.unwrap(),
    }))
}

#[get("/?<page>&<limit>")]
async fn list(
    conn: Connection<'_, Db>,
    page: Option<usize>,
    limit: Option<usize>,
) -> Result<RawJson<String>, Status> {
    let db = conn.into_inner();

    // Set page number and items per page
    let page = page.unwrap_or(1);
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if page == 0 || limit == 0 {
        return Err(Status::BadRequest);
    }

    // Setup paginator
    let paginator = Manga::find()
        .order_by_asc(manga::Column::Id)
        .paginate(db, limit);
    let num_pages = paginator.num_pages().await.ok().unwrap();

    // Fetch paginated manga
    let manga = paginator
        .fetch_page(page - 1)
        .await
        .expect("could not retrieve manga");

    Ok(RawJson(
        json! ({
            "page": page,
            "limit": limit,
            "num_pages": num_pages,
            "manga": manga,
        })
        .to_string(),
    ))
}

#[get("/<id>")]
async fn get(conn: Connection<'_, Db>, id: i32) -> Option<Json<manga::Model>> {
    let db = conn.into_inner();

    let manga = Manga::find_by_id(id).one(db).await.unwrap();

    if manga.is_some() {
        Some(Json(manga.unwrap()))
    } else {
        None
    }
}

#[delete("/<id>")]
async fn delete(conn: Connection<'_, Db>, id: i32) -> &'static str {
    let db = conn.into_inner();

    let manga: manga::ActiveModel = Manga::find_by_id(id).one(db).await.unwrap().unwrap().into();

    manga.delete(db).await.unwrap();

    "Manga successfully deleted"
}

pub fn routes() -> Vec<Route> {
    routes![create, delete, list, get]
}
