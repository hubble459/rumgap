use actix_web::error::{
    ErrorBadRequest, ErrorConflict, ErrorForbidden, ErrorInternalServerError, ErrorNotFound,
};
use actix_web::http::StatusCode;
use actix_web::{web, HttpResponse, Responder, Result};
use entity::manga::{ActiveModel as ActiveManga, Column as MangaColumn, Entity as manga};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QuerySelect};

use crate::api::v1::data;
use crate::api::v1::data::paginate::{Paginate, PaginateQuery};
use crate::middleware::auth::AuthService;

#[rustfmt::skip]
pub async fn get_manga_by_id(db: &DatabaseConnection, manga_id: i32) -> Result<data::manga::Full> {
    let found_user = manga::find_by_id(manga_id)
        .left_join(entity::chapter::Entity)
        .column_as(entity::chapter::Column::Id.count(), "count_chapters")
        .column_as(entity::chapter::Column::Posted.max(), "last_updated")
        .group_by(MangaColumn::Id)
        .into_model::<data::manga::Full>()
        .one(db)
        .await
        .map_err(ErrorInternalServerError)?
        .ok_or(ErrorNotFound("User not found"))?;

    Ok(found_user)
}

#[get("")]
async fn index(
    conn: web::Data<DatabaseConnection>,
    query: web::Query<PaginateQuery>,
) -> Result<Paginate<Vec<data::manga::Full>>> {
    let db = conn.as_ref();

    // Create paginate object
    let paginate = manga::find()
        .into_model::<data::manga::Full>()
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

#[post("")]
async fn store(
    conn: web::Data<DatabaseConnection>,
    data: web::Json<data::manga::Post>,
) -> Result<impl Responder> {
    let db = conn.as_ref();

    unimplemented!("owo");

    Ok((web::Json("owo"), StatusCode::CREATED))
}

#[patch("/{manga_id}")]
async fn edit(
    path: web::Path<u16>,
    conn: web::Data<DatabaseConnection>,
    auth: AuthService,
    data: web::Json<data::manga::Patch>,
) -> Result<impl Responder> {
    let manga_id = path.into_inner();
    let db = conn.as_ref();

    unimplemented!();

    Ok(web::Json(""))
}

#[get("/{manga_id}")]
async fn get(path: web::Path<u16>, conn: web::Data<DatabaseConnection>) -> Result<impl Responder> {
    let manga_id = path.into_inner();
    let db = conn.as_ref();

    let full_manga = get_manga_by_id(db, manga_id as i32).await?;

    Ok(web::Json(full_manga))
}

#[delete("/{manga_id}")]
async fn delete(
    path: web::Path<u16>,
    conn: web::Data<DatabaseConnection>,
    auth: AuthService,
) -> Result<impl Responder> {
    let manga_id = path.into_inner();

    if !auth.is_admin() {
        return Err(ErrorForbidden("You are not allowed to make this request"));
    }

    let db = conn.as_ref();

    let result = manga::delete_by_id(manga_id as i32)
        .exec(db)
        .await
        .map_err(ErrorInternalServerError)?;

    if result.rows_affected == 1 {
        Ok(HttpResponse::NoContent())
    } else {
        // Should never happen
        Err(ErrorNotFound("Manga not found!"))
    }
}

#[cfg(test)]
mod test {
    const TEST_USERNAME: &str = "test";
    const TEST_PASSWORD: &str = "P@ssw0rd!";
    const TEST_EMAIL: &str = "test@gmail.com";

    crate::test::test_resource! {
        user "/api/v1/user";

        post: "/" => StatusCode::CREATED; json!({"username": TEST_USERNAME, "email": TEST_EMAIL, "password": TEST_PASSWORD});;
        post: "/login" => StatusCode::CREATED; json!({"username": TEST_USERNAME, "password": TEST_PASSWORD});;
        get: "/";;
        get: "/" 0 id;;
        patch: "/" 0 id; json!({"username": "updated"}), AUTHORIZATION: "Bearer " 1 token;;
        delete: "/" 0 id => StatusCode::NO_CONTENT, AUTHORIZATION: "Bearer " 1 token;;
    }
}
