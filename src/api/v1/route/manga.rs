use actix_web::error::{
    ErrorBadRequest, ErrorConflict, ErrorForbidden, ErrorInternalServerError, ErrorNotFound,
};
use actix_web::http::StatusCode;
use actix_web::{web, HttpResponse, Responder, Result};
use chrono::{Duration, Utc};
use entity::manga::{ActiveModel as ActiveManga, Column as MangaColumn, Entity as manga};
use manga_parser::parser::{MangaParser, Parser};
use manga_parser::Url;
use migration::Expr;
use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::ActiveValue::NotSet;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QuerySelect, Set, QueryOrder,
};

use crate::api::v1::{data, util};
use crate::api::v1::data::paginate::Paginate;
use crate::api::v1::util::search::manga::lucene_filter;
use crate::middleware::auth::AuthService;

use super::chapter;

lazy_static! {
    pub static ref MANGA_PARSER: MangaParser = MangaParser::new();
}

pub const NEXT_UPDATE_QUERY: &str =
    "(MAX(chapter.posted) + (MAX(chapter.posted) - MIN(chapter.posted)) / NULLIF(COUNT(*) - 1, 0))";

#[rustfmt::skip]
pub async fn get_manga_by_id(db: &DatabaseConnection, manga_id: i32) -> Result<data::manga::Full> {
    let found_manga = manga::find_by_id(manga_id)
        .left_join(entity::chapter::Entity)
        .column_as(entity::chapter::Column::Id.count(), "count_chapters")
        .column_as(entity::chapter::Column::Posted.max(), "last")
        .column_as(Expr::cust(NEXT_UPDATE_QUERY), "next")
        .group_by(MangaColumn::Id)
        .into_model::<data::manga::Full>()
        .one(db)
        .await
        .map_err(ErrorInternalServerError)?
        .ok_or(ErrorNotFound("Manga not found"))?;

    Ok(found_manga)
}

pub async fn save_manga(
    db: &DatabaseConnection,
    id: Option<i32>,
    url: Url,
) -> actix_web::Result<data::manga::Full> {
    info!("Saving manga [{}]", url.to_string());

    let m = MANGA_PARSER
        .manga(url)
        .await
        .map_err(ErrorInternalServerError)?;

    let saved = ActiveManga {
        id: id.map_or(NotSet, |id| Set(id)),
        title: Set(m.title),
        description: Set(m.description),
        is_ongoing: Set(m.is_ongoing),
        cover: Set(m.cover.map(|url| url.to_string())),
        url: Set(m.url.to_string()),
        authors: Set(m.authors),
        alt_titles: Set(m.alt_titles),
        genres: Set(m.genres),
        ..Default::default()
    }
    .save(db)
    .await
    .map_err(ErrorInternalServerError)?;

    let manga_id = saved.id.unwrap();

    if m.chapters.is_empty() {
        error!("No chapters found for {} [{}]", manga_id, m.url.to_string());
    } else {
        // Remove old chapters
        let res = entity::chapter::Entity::delete_many()
            .filter(entity::chapter::Column::MangaId.eq(manga_id))
            .exec(db)
            .await
            .map_err(ErrorInternalServerError)?;
        if id.is_some() {
            info!("Cleared {} chapter(s)", res.rows_affected);
        }

        // Add new chapters
        let mut chapters = vec![];
        for chapter in m.chapters.iter() {
            chapters.push(entity::chapter::ActiveModel {
                manga_id: Set(manga_id),
                number: Set(chapter.number),
                url: Set(chapter.url.to_string()),
                title: Set(chapter.title.clone()),
                posted: Set(chapter.posted.map(|date| date.into())),
                ..Default::default()
            });
        }
        info!("Inserting {} chapter(s)", chapters.len());
        // Insert all in batch
        entity::chapter::Entity::insert_many(chapters)
            .exec_without_returning(db)
            .await
            .map_err(ErrorInternalServerError)?;
    }

    get_manga_by_id(db, manga_id).await
}

#[get("")]
async fn index(
    conn: web::Data<DatabaseConnection>,
    query: web::Query<data::manga::IndexQuery>,
) -> Result<Paginate<Vec<data::manga::Full>>> {
    let db = conn.as_ref();

    // Create paginate object
    let mut paginate = manga::find()
        .left_join(entity::chapter::Entity)
        .column_as(entity::chapter::Column::Id.count(), "count_chapters")
        .column_as(entity::chapter::Column::Posted.max(), "last")
        .column_as(Expr::cust(r#"(MAX("chapter"."posted") + (max(chapter.posted) - min(chapter.posted)) / nullif(count(*) - 1, 0))"#), "next")
        .group_by(MangaColumn::Id);

    if let Some(search) = query.search.clone() {
        paginate = paginate.having(lucene_filter(search.into())?);
    }

    if let Some(order) = query.order.clone() {
        let columns = util::order::manga::parse(&order)?;
        for (column, order) in columns {
            paginate = paginate.order_by(column, order);
        }
    }

    let paginate = paginate
        .into_model::<data::manga::Full>()
        .paginate(db, query.paginate.limit);

    // Get max page
    let amount = paginate
        .num_items_and_pages()
        .await
        .map_err(ErrorInternalServerError)?;

    // Get items from page
    let items = paginate
        .fetch_page(query.paginate.page)
        .await
        .map_err(ErrorInternalServerError)?;

    Ok(Paginate {
        total: amount.number_of_items,
        max_page: amount.number_of_pages,
        page: query.paginate.page,
        limit: query.paginate.limit,
        items,
    })
}

#[post("")]
async fn store(
    conn: web::Data<DatabaseConnection>,
    data: web::Json<data::manga::Post>,
    _auth: AuthService,
) -> Result<impl Responder> {
    let db = conn.into_inner();

    let urls: Result<Vec<Url>, _> = data
        .urls
        .clone()
        .into_iter()
        .map(|url| Url::parse(&url))
        .collect();
    let urls = urls.map_err(ErrorBadRequest)?;

    let handles = urls.into_iter().map(|url| {
        let db = db.clone();
        return actix_web::rt::spawn(async move {
            let db = db.clone();
            // Check for conflict
            manga::find()
                .filter(MangaColumn::Url.eq(url.clone().to_string()))
                .one(db.as_ref())
                .await
                .map_err(ErrorInternalServerError)?
                .map_or_else(
                    || Ok(()),
                    |m| {
                        Err(ErrorConflict(format!(
                            "Manga {} already exists with id {}",
                            m.url, m.id
                        )))
                    },
                )?;

            // TODO 11/12/2022: Group similar (alt) titles

            // Fetch and save manga
            return save_manga(&db, None, url).await;
        });
    });

    let handles = futures::future::join_all(handles).await;
    let mut mangas = vec![];

    for handle in handles {
        match handle {
            Ok(m) => {
                mangas.push(m?);
            }
            Err(e) => {
                return Err(ErrorInternalServerError(e));
            }
        }
    }

    Ok((web::Json(mangas), StatusCode::CREATED))
}

#[get("/{manga_id}")]
async fn get(path: web::Path<i32>, conn: web::Data<DatabaseConnection>) -> Result<impl Responder> {
    let manga_id = path.into_inner();
    let db = conn.as_ref();

    let (url, updated_at): (String, DateTimeWithTimeZone) = manga::find_by_id(manga_id.into())
        .select_only()
        .column(MangaColumn::Url)
        .column(MangaColumn::UpdatedAt)
        .into_values::<_, data::manga::Minimal>()
        .one(db)
        .await
        .map_err(ErrorInternalServerError)?
        .ok_or(ErrorNotFound("Manga not found"))?;

    let interval_ms: i64 = std::env::var("MANGA_UPDATE_INTERVAL_MS")
        .unwrap_or("3600000".to_string())
        .parse()
        .unwrap_or(3600000);

    // Check if it should be updated
    if (Utc::now() - Duration::milliseconds(interval_ms)) > updated_at {
        // Update
        info!("Updating manga with id '{}' [{}]", manga_id, url);
        let updated = save_manga(db, Some(manga_id.into()), Url::parse(&url).unwrap()).await?;
        return Ok(web::Json(updated));
    }

    let full_manga = get_manga_by_id(db, manga_id).await?;

    Ok(web::Json(full_manga))
}

#[delete("/{manga_id}")]
async fn delete(
    path: web::Path<i32>,
    conn: web::Data<DatabaseConnection>,
    auth: AuthService,
) -> Result<impl Responder> {
    let manga_id = path.into_inner();

    if !auth.is_admin() {
        return Err(ErrorForbidden("You are not allowed to make this request"));
    }

    let db = conn.as_ref();

    let result = manga::delete_by_id(manga_id)
        .exec(db)
        .await
        .map_err(ErrorInternalServerError)?;

    if result.rows_affected == 1 {
        Ok(HttpResponse::NoContent())
    } else {
        Err(ErrorNotFound("Manga not found!"))
    }
}

pub fn routes() -> actix_web::Scope {
    web::scope("/manga")
        .service(index)
        .service(store)
        .service(get)
        .service(delete)
        .service(chapter::routes())
}

#[cfg(test)]
mod test {

}
