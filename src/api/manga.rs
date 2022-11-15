use chrono::{Duration, Utc};
use parser::parser::{MangaParser, Parser};
use parser::Url;
use rocket::http::Status;
use rocket::response::content::RawJson;
use rocket::response::Redirect;
use rocket::serde::json::Json;
use rocket::serde::{Deserialize, Serialize};
use rocket::{Route, State};
use sea_orm::{entity::*, query::*, DatabaseBackend};
use sea_orm_rocket::Connection;

use entity::chapter;
use entity::chapter::Entity as Chapter;
use entity::manga::Entity as Manga;
use entity::manga::{self, SPLITTER};
use serde_json::json;

use crate::auth::User;
use crate::pagination::Pagination;
use crate::pool::Db;

pub const DEFAULT_LIMIT: u64 = 10;

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
) -> Result<Redirect, (Status, RawJson<JsonValue>)> {
    let db = conn.into_inner();

    let url = manga_url.into_inner().url;

    let exists = manga::Entity::find()
        .select_only()
        .column(manga::Column::Id)
        .filter(manga::Column::Url.eq(url.clone()))
        .one(db)
        .await
        .map_err(|e| {
            (
                Status::InternalServerError,
                RawJson(json!({"message": e.to_string()})),
            )
        })?;

    if exists.is_some() {
        let exists = exists.unwrap();
        let id = exists.id;
        return Ok(Redirect::to(format!("/api/manga/{}", id)));
    }

    let manga = parser
        .manga(Url::parse(&url).map_err(|e| {
            (
                Status::BadRequest,
                RawJson(json!({"message": e.to_string()})),
            )
        })?)
        .await
        .map_err(|e| {
            (
                Status::InternalServerError,
                RawJson(json!({"message": e.to_string()})),
            )
        })?;

    let stored = manga
        .clone()
        .into_active_model()
        .save(db)
        .await
        .map_err(|e| (Status::Conflict, RawJson(json!({"message": e.to_string()}))))?;

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
    .map_err(|e| {
        (
            Status::InternalServerError,
            RawJson(json!({"message": e.to_string()})),
        )
    })?;

    let id = stored.id.unwrap();
    Ok(Redirect::to(format!("/api/manga/{}", id)))
}

#[get("/?<page>&<limit>&<hide_reading>")]
async fn list(
    conn: Connection<'_, Db>,
    page: Option<u64>,
    limit: Option<u64>,
    hide_reading: Option<bool>,
    user: Option<User>,
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
    } else if hide_reading.is_some() && user.is_none() {
        return Err((
            Status::BadRequest,
            RawJson(
                json!({"message": "cannot hide reading if bearer token is missing in Authorization header"}),
            ),
        ));
    }

    // Setup paginator
    let paginator = Manga::find()
        .from_raw_sql(Statement::from_sql_and_values(
            DatabaseBackend::MySql,
            format!(r#"SELECT manga.*,
                          COUNT(c.manga_id) AS chapter_count,
                          DATE_ADD(
                            MAX(c.posted),
                            INTERVAL CAST(TIMESTAMPDIFF(SECOND, MIN(c.posted), MAX(c.posted)) / (COUNT(DISTINCT(c.posted)) - 1) AS UNSIGNED) SECOND
                          ) AS next_chapter,
                          MAX(c.posted) AS last_chapter
                          {}
                    FROM manga
                LEFT JOIN chapter AS c
                ON c.manga_id = manga.id
                {}
                GROUP BY IFNULL(c.manga_id, manga.id)
                {}"#,
                // If logged in, join reading and set boolean to true if user is reading the manga
                user.as_ref().map_or_else(|| "", |_u| ", IFNULL(reading.manga_id, 0) != 0 AS reading"),
                user.as_ref().map_or_else(|| "", |_u| "LEFT JOIN reading ON reading.user_id = ? AND reading.manga_id = manga.id"),
                // Hide reading
                hide_reading.map_or_else(|| "", |hide| if hide {
                    "HAVING reading = 0"
                } else {
                    ""
                }),
            ).as_str(),
            vec![user.map_or_else(|| 0, |u| u.id).into()],
        ))
        .into_json()
        .paginate(db, limit);
    let num_pages = paginator.num_pages().await.map_err(|e| {
        (
            Status::InternalServerError,
            RawJson(json!({"message": e.to_string()})),
        )
    })?;

    let num_items = paginator.num_items().await.map_err(|e| {
        (
            Status::InternalServerError,
            RawJson(json!({"message": e.to_string()})),
        )
    })?;

    // Fetch paginated manga
    let manga: Vec<JsonValue> = paginator
        .fetch_page(page - 1)
        .await
        .map_err(|e| {
            (
                Status::BadRequest,
                RawJson(json!({"message": e.to_string()})),
            )
        })?
        .into_iter()
        .map(map_manga_json)
        .collect();

    Ok(Json(Pagination {
        page,
        limit,
        num_items,
        num_pages,
        items: manga,
    }))
}

#[get("/<id>?<force_refresh>")]
async fn get(
    conn: Connection<'_, Db>,
    id: u32,
    force_refresh: Option<bool>,
    user: Option<User>,
    parser: &State<MangaParser>,
) -> Result<Option<Json<JsonValue>>, (Status, RawJson<JsonValue>)> {
    let db = conn.into_inner();

    let old_manga = Manga::find_by_id(id).one(db).await.unwrap();

    if old_manga.is_none() {
        return Ok(None);
    }
    let old_manga = old_manga.unwrap();
    let force_refresh = force_refresh.unwrap_or(false);

    let diff = Utc::now().timestamp_millis() - old_manga.updated_at.timestamp_millis();
    if force_refresh || diff > Duration::minutes(10).num_milliseconds() {
        // Update manga because it is 10 minutes old
        println!("[UPDATING] {}", old_manga.title);

        let url = old_manga.url;

        let manga = parser
            .manga(Url::parse(&url).map_err(|e| {
                (
                    Status::BadRequest,
                    RawJson(json!({ "error": e.to_string() })),
                )
            })?)
            .await
            .map_err(|e| {
                (
                    Status::InternalServerError,
                    RawJson(json!({ "error": e.to_string() })),
                )
            })?;

        let mut new_manga = manga.clone().into_active_model();
        new_manga.id = ActiveValue::Set(id);
        new_manga.created_at = ActiveValue::Set(old_manga.created_at);
        new_manga.updated_at = ActiveValue::Set(Utc::now());

        let stored = new_manga.save(db).await.map_err(|e| {
            (
                Status::InternalServerError,
                RawJson(json!({ "error": e.to_string() })),
            )
        })?;

        // Clear old chapters
        Chapter::delete_many()
            .filter(chapter::Column::MangaId.eq(id))
            .exec(db)
            .await
            .map_err(|e| {
                (
                    Status::InternalServerError,
                    RawJson(json!({ "error": e.to_string() })),
                )
            })?;

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
        .map_err(|e| {
            (
                Status::InternalServerError,
                RawJson(json!({ "error": e.to_string() })),
            )
        })?;
    }

    let manga = Manga::find()
    .from_raw_sql(Statement::from_sql_and_values(
        DatabaseBackend::MySql,
        format!(r#"SELECT manga.*,
                      COUNT(c.manga_id) as chapter_count,
                      DATE_ADD(
                        MAX(c.posted),
                        INTERVAL CAST(TIMESTAMPDIFF(SECOND, MIN(c.posted), MAX(c.posted)) / (COUNT(DISTINCT(c.posted)) - 1) AS UNSIGNED) SECOND
                      ) as next_chapter,
                      MAX(c.posted) AS last_chapter
                      {}
                FROM manga
            LEFT JOIN chapter AS c
            ON c.manga_id = manga.id
            {}
            WHERE manga.id = ?"#,
            user.as_ref().map_or_else(|| "", |_u| ", IFNULL(reading.manga_id, 0) != 0 AS reading"),
            user.as_ref().map_or_else(|| "".to_owned(), |u| format!("LEFT JOIN reading ON reading.user_id = {} AND reading.manga_id = manga.id", u.id)),
        ).as_str(),
        vec![id.into()],
    ))
        .into_json()
        .one(db)
        .await
        .map_err(|e| (
            Status::InternalServerError,
            RawJson(json!({ "error": e.to_string() })),
        ))?;

    Ok(manga.map(|manga| Json(map_manga_json(manga))))
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
