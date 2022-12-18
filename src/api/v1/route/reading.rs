use actix_web::error::{ErrorForbidden, ErrorInternalServerError, ErrorNotFound};
use actix_web::http::StatusCode;
use actix_web::{web, HttpResponse, Responder, Result};
use entity::reading::{
    ActiveModel as ActiveReading, Column as ReadingColumn, Entity as reading_entity,
};
use migration::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, RelationTrait, Set,
};

use crate::api::v1::data::paginate::Paginate;
use crate::api::v1::route::manga::NEXT_UPDATE_QUERY;
use crate::api::v1::util::search::reading::lucene_filter;
use crate::api::v1::{data, util};
use crate::middleware::auth::AuthService;

#[rustfmt::skip]
pub async fn get_reading_by_id(db: &DatabaseConnection, user_id: i32, reading_id: i32) -> Result<serde_json::Value> {
    let (reading, manga) = reading_entity::find_by_id(reading_id)
        .filter(ReadingColumn::UserId.eq(user_id))
        .left_join(entity::manga::Entity)
        .select_also(entity::manga::Entity)
        .join(
            migration::JoinType::LeftJoin,
            entity::manga::Relation::Chapter.def(),
        )
        .column_as(entity::chapter::Column::Id.count(), "B_count_chapters")
        .column_as(entity::chapter::Column::Posted.max(), "B_last")
        .column_as(Expr::cust(NEXT_UPDATE_QUERY), "B_next")
        .group_by(entity::manga::Column::Id)
        .group_by(ReadingColumn::Id)
        .into_model::<data::reading::Full, data::manga::Full>()
        .one(db)
        .await
        .map_err(ErrorInternalServerError)?
        .ok_or(ErrorNotFound("Chapter not found"))?;

    let mut found = json!(reading);
    found["manga"] = json!(manga);
    Ok(found)
}

#[get("")]
async fn index(
    conn: web::Data<DatabaseConnection>,
    query: web::Query<data::reading::IndexQuery>,
    auth: AuthService,
) -> Result<Paginate<Vec<serde_json::Value>>> {
    let db = conn.as_ref();

    // Create paginate object
    let mut paginate = reading_entity::find()
        .filter(ReadingColumn::UserId.eq(auth.user.id))
        .left_join(entity::manga::Entity)
        .select_also(entity::manga::Entity)
        .join(
            migration::JoinType::LeftJoin,
            entity::manga::Relation::Chapter.def(),
        )
        .column_as(entity::chapter::Column::Id.count(), "B_count_chapters")
        .column_as(entity::chapter::Column::Posted.max(), "B_last")
        .column_as(Expr::cust(NEXT_UPDATE_QUERY), "B_next")
        .group_by(entity::manga::Column::Id)
        .group_by(ReadingColumn::Id);

    if let Some(search) = query.search.clone() {
        paginate = paginate.having(lucene_filter(search.into())?);
    }

    if let Some(order) = query.order.clone() {
        let columns = util::order::reading::parse(&order)?;
        for (column, order) in columns {
            paginate = paginate.order_by(column, order);
        }
    }

    let paginate = paginate
        .into_model::<data::reading::Full, data::manga::Full>()
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
        items: items
            .into_iter()
            .map(|(reading, manga)| {
                let mut reading = json!(reading);
                reading["manga"] = json!(manga);
                reading
            })
            .collect(),
    })
}

#[post("")]
async fn store(
    conn: web::Data<DatabaseConnection>,
    data: web::Json<data::reading::Post>,
    auth: AuthService,
) -> Result<impl Responder> {
    let db = conn.into_inner();

    let saved = ActiveReading {
        manga_id: Set(data.manga_id),
        user_id: Set(auth.user.id),
        ..Default::default()
    }
    .insert(db.clone().as_ref())
    .await
    .map_err(ErrorInternalServerError)?;

    let r = get_reading_by_id(&db, saved.user_id, saved.id).await?;

    Ok((web::Json(r), StatusCode::CREATED))
}

#[get("/{reading_id}")]
async fn get(
    path: web::Path<i32>,
    conn: web::Data<DatabaseConnection>,
    auth: AuthService,
) -> Result<impl Responder> {
    let reading_id = path.into_inner();
    let db = conn.as_ref();

    let full_reading = get_reading_by_id(db, auth.user.id, reading_id).await?;

    Ok(web::Json(full_reading))
}

#[patch("/{reading_id}")]
async fn patch(
    path: web::Path<i32>,
    conn: web::Data<DatabaseConnection>,
    data: web::Json<data::reading::Patch>,
    auth: AuthService,
) -> Result<impl Responder> {
    let reading_id = path.into_inner();
    let db = conn.as_ref();

    reading_entity::find_by_id(reading_id)
        .filter(ReadingColumn::UserId.eq(auth.user.id))
        .one(db)
        .await
        .map_err(ErrorInternalServerError)?
        .ok_or(ErrorNotFound("Reading not found"))?;

    ActiveReading {
        id: Set(reading_id),
        progress: Set(data.progress),
        ..Default::default()
    }
    .insert(db)
    .await
    .map_err(ErrorInternalServerError)?;

    let full_reading = get_reading_by_id(db, auth.user.id, reading_id).await?;

    Ok(web::Json(full_reading))
}

#[delete("/{manga_id}")]
async fn delete(
    path: web::Path<i32>,
    conn: web::Data<DatabaseConnection>,
    auth: AuthService,
) -> Result<impl Responder> {
    let reading_id = path.into_inner();

    if !auth.is_admin() {
        return Err(ErrorForbidden("You are not allowed to make this request"));
    }

    let db = conn.as_ref();

    let result = reading_entity::delete_by_id(reading_id)
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
    web::scope("/reading")
        .service(index)
        .service(store)
        .service(patch)
        .service(get)
        .service(delete)
}

#[cfg(test)]
mod test {}
