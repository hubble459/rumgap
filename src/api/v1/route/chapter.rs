use crate::api::v1::data;
use crate::api::v1::data::paginate::Paginate;
use crate::api::v1::route::manga::MANGA_PARSER;
use actix_web::error::{ErrorInternalServerError, ErrorNotFound};
use actix_web::{web, Responder, Result};
use entity::chapter::{Column as ChapterColumn, Entity as chapter, Model as Chapter};
use manga_parser::parser::Parser;
use manga_parser::Url;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
};

#[get("")]
async fn index(
    path: web::Path<i32>,
    conn: web::Data<DatabaseConnection>,
    query: web::Query<data::paginate::PaginateQuery>,
) -> Result<Paginate<Vec<Chapter>>> {
    let db = conn.as_ref();
    let manga_id = path.into_inner();

    // Create paginate object
    let paginate = chapter::find()
        .filter(ChapterColumn::MangaId.eq(manga_id))
        .order_by(ChapterColumn::Id, migration::Order::Asc)
        .paginate(db, query.limit);

    // Get max page
    let amount = paginate
        .num_items_and_pages()
        .await
        .map_err(ErrorInternalServerError)?;

    // Get items from page
    let items = paginate
        .fetch_page(query.page)
        .await
        .map_err(ErrorInternalServerError)?;

    Ok(Paginate {
        total: amount.number_of_items,
        max_page: amount.number_of_pages,
        page: query.page,
        limit: query.limit,
        items,
    })
}

#[get("/{chapter_id}")]
async fn get(
    path: web::Path<(i32, i32)>,
    conn: web::Data<DatabaseConnection>,
) -> Result<impl Responder> {
    let (manga_id, chapter_id) = path.into_inner();
    let db = conn.as_ref();

    // Get chapter
    let chap = chapter::find()
        .filter(ChapterColumn::Id.eq(chapter_id))
        .filter(ChapterColumn::MangaId.eq(manga_id))
        .one(db)
        .await
        .map_err(ErrorInternalServerError)?
        .ok_or(ErrorNotFound("Manga not found"))?;

    // Get images
    let images = MANGA_PARSER
        .images(&Url::parse(&chap.url).unwrap())
        .await
        .map_err(ErrorInternalServerError)?;

    Ok(web::Json(images))
}

pub fn routes() -> actix_web::Scope {
    web::scope("/{manga_id}/chapter")
        .service(index)
        .service(get)
}

#[cfg(test)]
mod test {}
