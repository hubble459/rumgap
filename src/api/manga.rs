use rocket::form::Form;
use rocket::http::Status;
use rocket::response::content::RawJson;
use rocket::serde::json::Json;
use rocket::Route;
use sea_orm::{entity::*, query::*};
use sea_orm_rocket::Connection;
use serde_json::json;

use entity::manga;
use entity::manga::Entity as Manga;

use crate::pool::Db;

const DEFAULT_LIMIT: usize = 5;

#[post("/", data = "<manga_form>")]
async fn create(conn: Connection<'_, Db>, manga_form: Json<manga::Model>) -> Json<manga::Model> {
    let db = conn.into_inner();

    let form = manga_form.into_inner();

    let stored = manga::ActiveModel {
        title: Set(form.title.to_owned()),
        description: Set(form.description.to_owned()),
        ..Default::default()
    }
    .save(db)
    .await
    .expect("could not insert manga");

    Json(manga::Model {
        id: stored.id.unwrap(),
        title: stored.title.unwrap(),
        description: stored.description.unwrap(),
    })
}

#[post("/<id>", data = "<manga_form>")]
async fn update(
    conn: Connection<'_, Db>,
    id: i32,
    manga_form: Form<manga::Model>,
) -> Json<manga::Model> {
    let db = conn.into_inner();

    let manga: manga::ActiveModel = Manga::find_by_id(id).one(db).await.unwrap().unwrap().into();

    let form = manga_form.into_inner();

    Json(
        db.transaction::<_, manga::Model, sea_orm::DbErr>(|txn| {
            Box::pin(async move {
                let manga = manga::ActiveModel {
                    id: manga.id,
                    title: Set(form.title.to_owned()),
                    description: Set(form.description.to_owned()),
                }
                .save(txn)
                .await
                .expect("could not edit manga");

                Ok(manga::Model {
                    id: manga.id.unwrap(),
                    title: manga.title.unwrap(),
                    description: manga.description.unwrap(),
                })
            })
        })
        .await
        .unwrap(),
    )
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
    if page == 0 || limit == 0  {
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
    routes![create, delete, list, get, update]
}
