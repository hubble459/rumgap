use rocket::serde::json::Json;
use rocket::{http::Status, Route};
use sea_orm::PaginatorTrait;
use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};
use sea_orm_rocket::Connection;
use serde_json::Value;

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
    page: Option<usize>,
    limit: Option<usize>,
) -> Result<Json<Pagination<Vec<Value>>>, Status> {
    let page = page.unwrap_or(1);
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if page == 0 || limit == 0 {
        return Err(Status::BadRequest);
    }

    let db = conn.into_inner();

    let paginator = Chapter::find()
        .filter(chapter::Column::MangaId.eq(manga_id))
        .order_by_asc(chapter::Column::Number)
        .order_by_asc(chapter::Column::Posted)
        .into_json()
        .paginate(db, limit);
    let num_pages = paginator.num_pages().await.ok().unwrap();

    let chapters = paginator
        .fetch_page(page - 1)
        .await
        .map_err(|_| Status::InternalServerError)?;

    Ok(Json(Pagination {
        num_pages,
        page,
        limit,
        data: chapters,
    }))
}

#[get("/<_manga_id>/chapter/<chapter_id>")]
async fn get(
    conn: Connection<'_, Db>,
    _manga_id: u32,
    chapter_id: u32,
) -> Result<Json<serde_json::Value>, Status> {
    let db = conn.into_inner();

    let chapter = Chapter::find_by_id(chapter_id)
        .into_json()
        .one(db)
        .await
        .map_err(|_| Status::InternalServerError)?
        .ok_or(Status::NotFound)?;

    Ok(Json(chapter))
}

pub fn routes() -> Vec<Route> {
    routes![index, get]
}

pub fn base() -> &'static str {
    "/manga/"
}
