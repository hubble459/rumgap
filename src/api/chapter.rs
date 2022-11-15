use parser::parser::{MangaParser, Parser};
use parser::Url;
use rocket::response::content::RawJson;
use rocket::serde::json::Json;
use rocket::State;
use rocket::{http::Status, Route};
use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};
use sea_orm::{PaginatorTrait, QuerySelect};
use sea_orm_rocket::Connection;
use serde_json::{json, Value};

use entity::chapter;
use entity::chapter::Entity as Chapter;
use sea_orm::EntityTrait;

use crate::pagination::Pagination;
use crate::pool::Db;

use super::manga::DEFAULT_LIMIT;

#[get("/<manga_id>/chapter?<page>&<limit>")]
async fn index(
    conn: Connection<'_, Db>,
    manga_id: u32,
    page: Option<u64>,
    limit: Option<u64>,
) -> Result<Json<Pagination<Vec<Value>>>, Status> {
    let page = page.unwrap_or(1);
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if page == 0 || limit == 0 {
        return Err(Status::BadRequest);
    }

    let db = conn.into_inner();

    // IFNULL(NULLIF(title, ''), CONCAT('Chapter ', chapter.number))
    let paginator = Chapter::find()
        .filter(chapter::Column::MangaId.eq(manga_id))
        .order_by_desc(chapter::Column::Number)
        .order_by_desc(chapter::Column::Posted)
        .into_json()
        .paginate(db, limit);
    let num_pages = paginator.num_pages().await.ok().unwrap();
    let num_items = paginator.num_items().await.ok().unwrap();

    let chapters = paginator
        .fetch_page(page - 1)
        .await
        .map_err(|_| Status::InternalServerError)?;

    Ok(Json(Pagination {
        num_pages,
        num_items,
        page,
        limit,
        items: chapters
            .into_iter()
            .enumerate()
            .map(|(index, mut chapter)| {
                chapter["index"] = json!(num_items - (index as u64 + (page - 1) * limit));
                let title = chapter["title"].as_str().unwrap();
                if title.is_empty() {
                    let number = chapter["number"].as_f64().unwrap();
                    chapter["title"] = json!("Chapter ".to_owned() + &number.to_string())
                }
                chapter
            })
            .collect(),
    }))
}

#[get("/<manga_id>/chapter/<index>")]
async fn get(
    conn: Connection<'_, Db>,
    manga_id: u32,
    index: u32,
) -> Result<Json<Value>, (Status, RawJson<serde_json::Value>)> {
    if index < 1 {
        return Err((
            Status::BadRequest,
            RawJson(json!({"message": "Progress should be bigger than 1"})),
        ));
    }

    let db = conn.into_inner();

    let mut chapter = Chapter::find()
        .filter(chapter::Column::MangaId.eq(manga_id))
        .offset((index - 1).into())
        .order_by_asc(chapter::Column::Number)
        .order_by_asc(chapter::Column::Posted)
        .limit(1)
        .into_json()
        .one(db)
        .await
        .map_err(|e| {
            (
                Status::InternalServerError,
                RawJson(json!({"message": e.to_string()})),
            )
        })?
        .ok_or((
            Status::NotFound,
            RawJson(json!({"message": "Chapter not found"})),
        ))?;

    chapter["index"] = json!(index);

    Ok(Json(chapter))
}

#[get("/<_manga_id>/chapter/<chapter_id>/images")]
async fn images(
    conn: Connection<'_, Db>,
    _manga_id: u32,
    chapter_id: u32,
    parser: &State<MangaParser>,
) -> Result<Json<Vec<Url>>, (Status, RawJson<serde_json::Value>)> {
    let db = conn.into_inner();

    let chapter = Chapter::find_by_id(chapter_id)
        .one(db)
        .await
        .map_err(|e| {
            (
                Status::InternalServerError,
                RawJson(json!({"message": e.to_string()})),
            )
        })?
        .ok_or((
            Status::NotFound,
            RawJson(json!({"message": "Chapter not found"})),
        ))?;

    let images = parser
        .images(Url::parse(&chapter.url).unwrap())
        .await
        .map_err(|e| {
            (
                Status::BadRequest,
                RawJson(json!({"message": e.to_string()})),
            )
        })?;

    Ok(Json(images))
}

pub fn routes() -> Vec<Route> {
    routes![index, get, images]
}

pub fn base() -> &'static str {
    "/manga/"
}
