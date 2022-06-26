use entity::reading;
use entity::reading::ActiveModel as ActiveReading;
use entity::reading::Entity as Reading;
use rocket::response::content::RawJson;
use rocket::serde::{Deserialize, Serialize};
use rocket::{http::Status, serde::json::Json, Route};
use sea_orm::{entity::*, query::*, DatabaseConnection};
use sea_orm_rocket::Connection;
use serde_json::json;

use crate::{auth::User, pagination::Pagination, pool::Db};

use super::manga::DEFAULT_LIMIT;

#[get("/?<page>&<limit>&<keyword>")]
async fn index(
    conn: Connection<'_, Db>,
    page: Option<usize>,
    limit: Option<usize>,
    keyword: Option<String>,
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

    let keyword = keyword.unwrap_or("".to_owned());

    let keyword = regex::Regex::new(r"([^\w\d ])")
        .unwrap()
        .replace_all(keyword.trim(), " ")
        .to_string();

    let keyword = keyword
        .split_whitespace()
        .map(|word| format!("+{}*", word))
        .collect::<Vec<String>>()
        .join(" ")
        .to_owned();

    println!("{}", &keyword);

    let paginator = Reading::find()
        .from_raw_sql(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::MySql,
            format!(r#"SELECT reading.*,
                        manga.url, manga.title, manga.ongoing, manga.cover,
                        manga.created_at AS manga_created_at,
                        manga.updated_at AS manga_updated_at,
                        COUNT(c.manga_id) as chapter_count,
                        DATE_ADD(
                            MAX(c.posted),
                            INTERVAL CAST(TIMESTAMPDIFF(SECOND, MIN(c.posted), MAX(c.posted)) / (COUNT(DISTINCT(c.posted)) - 1) AS UNSIGNED) SECOND
                        ) as next_chapter,
                        MAX(c.posted) AS last_chapter
                    FROM reading
                LEFT JOIN manga
                ON manga.id = reading.manga_id
                LEFT JOIN chapter AS c
                ON c.manga_id = manga.id
                WHERE reading.user_id = ?
                {}
                GROUP BY c.manga_id
                ORDER BY manga.updated_at DESC"#,
                if keyword.is_empty() {
                    ""
                } else {
                    "AND MATCH(manga.title, manga.description, manga.genres, manga.authors, manga.alt_titles)
                AGAINST (? IN BOOLEAN MODE)"
            }).as_str(),
            vec![
                user.id.into(),
                keyword.into()
            ],
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
        num_items,
        data: reading
            .into_iter()
            .map(|value| {
                json!({
                    "progress": value["progress"],
                    "manga": {
                        "id": value["manga_id"],
                        "updated_at": value["manga_updated_at"],
                        "created_at": value["manga_created_at"],
                        "chapter_count": value["chapter_count"],
                        "next_chapter": value["next_chapter"],
                        "last_chapter": value["last_chapter"],
                        "cover": value["cover"],
                        "url": value["url"],
                        "ongoing": value["ongoing"],
                        "title": value["title"]
                    }
                })
            })
            .collect(),
    }))
}

async fn get_reading(
    db: &DatabaseConnection,
    id: (u32, u32),
) -> Result<Option<Json<JsonValue>>, (Status, RawJson<JsonValue>)> {
    let reading_value = Reading::find()
    .from_raw_sql(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::MySql,
        r#"SELECT reading.*,
                    manga.url, manga.title, manga.ongoing, manga.cover,
                    manga.created_at AS manga_created_at,
                    manga.updated_at AS manga_updated_at,
                    COUNT(c.manga_id) as chapter_count,
                    DATE_ADD(
                        MAX(c.posted),
                        INTERVAL CAST(TIMESTAMPDIFF(SECOND, MIN(c.posted), MAX(c.posted)) / (COUNT(DISTINCT(c.posted)) - 1) AS UNSIGNED) SECOND
                    ) as next_chapter,
                    MAX(c.posted) AS last_chapter
                FROM reading
            LEFT JOIN manga
            ON manga.id = reading.manga_id
            LEFT JOIN chapter AS c
            ON c.manga_id = manga.id
            WHERE reading.manga_id = ?
            AND reading.user_id = ?
            GROUP BY c.manga_id
            ORDER BY manga.updated_at DESC"#,
        vec![id.0.into(), id.1.into()],
    ))
    .into_json()
    .one(db)
    .await
    .map_err(|e| {
        (
            Status::InternalServerError,
            RawJson(json!({"message": e.to_string()})),
        )
    })?;

    Ok(reading_value.map(|value| {
        Json(json!({
            "progress": value["progress"],
            "manga": {
                "id": value["manga_id"],
                "updated_at": value["manga_updated_at"],
                "created_at": value["manga_created_at"],
                "chapter_count": value["chapter_count"],
                "next_chapter": value["next_chapter"],
                "last_chapter": value["last_chapter"],
                "cover": value["cover"],
                "url": value["url"],
                "ongoing": value["ongoing"],
                "title": value["title"]
            }
        }))
    }))
}

#[get("/<id>")]
async fn get(
    conn: Connection<'_, Db>,
    id: u32,
    user: User,
) -> Result<Option<Json<JsonValue>>, (Status, RawJson<JsonValue>)> {
    let db = conn.into_inner();

    get_reading(db, (id, user.id)).await
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
) -> Result<Option<Json<JsonValue>>, (Status, RawJson<JsonValue>)> {
    let manga_id = manga.manga_id;

    let db = conn.into_inner();

    let reading = ActiveReading {
        manga_id: ActiveValue::Set(manga_id),
        user_id: ActiveValue::Set(user.id),
        progress: ActiveValue::Set(0),
        ..Default::default()
    };

    let inserted = Reading::insert(reading).exec(db).await.map_err(|e| {
        (
            Status::BadRequest,
            RawJson(json!({"message": e.to_string()})),
        )
    })?;

    get_reading(db, inserted.last_insert_id).await
}

#[delete("/<id>")]
async fn delete(
    conn: Connection<'_, Db>,
    id: u32,
    user: User,
) -> Result<Status, (Status, RawJson<JsonValue>)> {
    let db = conn.into_inner();

    let result = Reading::delete_by_id((id, user.id))
        .filter(reading::Column::UserId.eq(user.id))
        .exec(db)
        .await
        .map_err(|e| {
            (
                Status::BadRequest,
                RawJson(json!({"message": e.to_string()})),
            )
        })?;

    Ok(if result.rows_affected == 1 {
        Status::NoContent
    } else {
        Status::NotFound
    })
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
        manga_id: ActiveValue::Set(id),
        user_id: ActiveValue::Set(user.id),
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
    routes![index, post, delete, patch, get]
}

pub fn base() -> &'static str {
    "reading"
}
