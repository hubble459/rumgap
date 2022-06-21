use rocket::response::content::RawJson;
use rocket::serde::{Deserialize, Serialize};
use rocket::{http::Status, serde::json::Json, Route};
use sea_orm::{entity::*, query::*};
use sea_orm_rocket::Connection;
use serde_json::json;

use entity::chapter;
use entity::manga;
use entity::reading;
use entity::reading::ActiveModel as ActiveReading;
use entity::reading::Entity as Reading;

use crate::{auth::User, pagination::Pagination, pool::Db};

use super::manga::DEFAULT_LIMIT;

#[get("/?<page>&<limit>")]
async fn index(
    conn: Connection<'_, Db>,
    page: Option<usize>,
    limit: Option<usize>,
    user: User,
) -> Result<Json<Pagination<Vec<JsonValue>>>, (Status, RawJson<JsonValue>)> {
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

    let db = conn.into_inner();

    let paginator = Reading::find()
        .filter(reading::Column::UserId.eq(user.id))
        .left_join(manga::Entity)
        .column(manga::Column::Url)
        .column_as(manga::Column::CreatedAt, "manga_created_at")
        .column_as(manga::Column::UpdatedAt, "manga_updated_at")
        .column(manga::Column::Title)
        .column(manga::Column::Ongoing)
        .column(manga::Column::Cover)
        .join_rev(JoinType::LeftJoin, chapter::Relation::Manga.def())
        .column_as(chapter::Column::MangaId.count(), "chapter_count")
        .order_by_asc(manga::Column::Title)
        .into_json()
        .paginate(db, limit);
    let num_pages = paginator.num_pages().await.map_err(|e| {
        (
            Status::InternalServerError,
            RawJson(json!({"message": e.to_string()})),
        )
    })?;

    let reading = paginator.fetch_page(page - 1).await.map_err(|e| {
        (
            Status::InternalServerError,
            RawJson(json!({"message": e.to_string()})),
        )
    })?;

    Ok(Json(Pagination {
        num_pages,
        page,
        limit,
        data: reading
            .into_iter()
            .map(|value| {
                json!({
                    "id": value["id"],
                    "progress": value["progress"],
                    "manga": {
                        "id": value["manga_id"],
                        "updated_at": value["manga_updated_at"],
                        "created_at": value["manga_created_at"],
                        "chapter_count": value["chapter_count"],
                        "cover": value["cover"],
                        "url": value["url"],
                        "ongoing": value["ongoing"],
                        "title": value["title"]
                    }
                })
            })
            .filter(|v| v["id"] != json!(null))
            .collect(),
    }))
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "rocket::serde")]
pub struct MangaData {
    manga_id: u32,
}

#[post("/", data = "<manga>")]
async fn post(
    conn: Connection<'_, Db>,
    manga: Json<MangaData>,
    user: User,
) -> Result<Json<reading::Model>, (Status, RawJson<JsonValue>)> {
    let manga_id = manga.manga_id;

    let db = conn.into_inner();

    let reading = ActiveReading {
        manga_id: ActiveValue::Set(manga_id),
        user_id: ActiveValue::Set(user.id),
        progress: ActiveValue::Set(0),
        ..Default::default()
    };

    let reading = Reading::insert(reading)
        .exec_with_returning(db)
        .await
        .map_err(|e| {
            (
                Status::BadRequest,
                RawJson(json!({"message": e.to_string()})),
            )
        })?;

    Ok(Json(reading))
}

#[delete("/<id>")]
async fn delete(
    conn: Connection<'_, Db>,
    id: u32,
    user: User,
) -> Result<Status, (Status, RawJson<JsonValue>)> {
    let db = conn.into_inner();

    Reading::delete_by_id(id)
        .filter(reading::Column::UserId.eq(user.id))
        .exec(db)
        .await
        .map_err(|e| {
            (
                Status::BadRequest,
                RawJson(json!({"message": e.to_string()})),
            )
        })?;

    Ok(Status::NoContent)
}

#[derive(Deserialize, Serialize)]
#[serde(crate = "rocket::serde")]
pub struct ProgressData {
    progress: u32,
}

#[patch("/<id>", data = "<progress_data>")]
async fn patch(
    conn: Connection<'_, Db>,
    id: u32,
    progress_data: Json<ProgressData>,
    user: User,
) -> Result<Json<reading::Model>, (Status, RawJson<JsonValue>)> {
    let db = conn.into_inner();

    let edit = ActiveReading {
        id: ActiveValue::Set(id),
        progress: ActiveValue::Set(progress_data.progress),
        ..Default::default()
    };

    let reading = Reading::update(edit)
        .filter(reading::Column::UserId.eq(user.id))
        .exec(db)
        .await
        .map_err(|e| {
            (
                Status::BadRequest,
                RawJson(json!({"message": e.to_string()})),
            )
        })?;

    Ok(Json(reading))
}

pub fn routes() -> Vec<Route> {
    routes![index, post, delete, patch]
}

pub fn base() -> &'static str {
    "reading"
}
