use actix_web::error::{ErrorBadGateway, ErrorConflict, ErrorInternalServerError, ErrorNotFound};
use actix_web::{web, HttpResponse, Responder, Result};
use entity::user::{
    ActiveModel as ActiveUser, Column as UserColumn, Entity as user, Model as User,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, IntoActiveModel,
    PaginatorTrait, QueryFilter, Set,
};
use serde_json::json;

use crate::api::v1::data;
use crate::api::v1::data::paginate::{Paginate, PaginateQuery};
use crate::api::v1::util::{encrypt, permission, verify};
use crate::middleware::auth::{sign, AuthService};

#[get("/")]
async fn index(
    conn: web::Data<DatabaseConnection>,
    query: web::Query<PaginateQuery>,
) -> Result<Paginate<Vec<User>>> {
    let db = conn.as_ref();

    // Create paginate object
    let paginate = user::find().paginate(db, query.limit);

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

#[post("/")]
async fn store(
    conn: web::Data<DatabaseConnection>,
    data: web::Json<data::user::Post>,
) -> Result<impl Responder> {
    let new_user = ActiveUser {
        username: Set(verify::username(&data.username)?),
        email: Set(verify::email(&data.email)?),
        password_hash: Set(encrypt::encrypt(&verify::password(&data.email)?)?),
        ..Default::default()
    };

    let db = conn.as_ref();

    let created = new_user
        .insert(db)
        .await
        .map_err(ErrorInternalServerError)?;

    Ok(web::Json(created))
}

#[patch("/{user_id}")]
async fn edit(
    path: web::Path<u16>,
    conn: web::Data<DatabaseConnection>,
    auth: AuthService,
    data: web::Json<data::user::Patch>,
) -> Result<impl Responder> {
    let user_id = path.into_inner();

    permission::can_edit(auth, user_id as i32)?;

    let db = conn.as_ref();

    let found_user = user::find_by_id(user_id as i32)
        .one(db)
        .await
        .map_err(ErrorInternalServerError)?
        .ok_or(ErrorNotFound("User not found"))?;

    let mut edit_user = found_user.into_active_model();

    if let Some(username) = data.0.username {
        edit_user.username = Set(verify::username(&username)?);
    }
    if let Some(email) = data.0.email {
        edit_user.email = Set(verify::email(&email)?);
    }
    if let Some(password) = data.0.password {
        let password = verify::password(&password)?;
        let password_hash = encrypt::encrypt(&password)?;
        edit_user.password_hash = Set(password_hash);
    }

    let created = edit_user.update(db).await.map_err(|e| {
        error!("{:#?}", e);
        ErrorConflict(e)
    })?;

    Ok(web::Json(created))
}

#[delete("/{user_id}")]
async fn delete(
    path: web::Path<u16>,
    conn: web::Data<DatabaseConnection>,
    auth: AuthService,
) -> Result<impl Responder> {
    let user_id = path.into_inner();

    permission::can_edit(auth, user_id as i32)?;

    let db = conn.as_ref();

    let result = user::delete_by_id(user_id as i32)
        .exec(db)
        .await
        .map_err(ErrorInternalServerError)?;

    if result.rows_affected == 1 {
        Ok(HttpResponse::NoContent())
    } else {
        // Should never happen
        Err(ErrorNotFound("User not found!"))
    }
}

#[post("/login")]
async fn login(
    conn: web::Data<DatabaseConnection>,
    data: web::Json<data::user::Login>,
) -> Result<impl Responder> {
    let error = "Username and password mismatch";
    let db = conn.as_ref();

    let found;

    if let Some(username) = data.0.username {
        found = user::find()
            .filter(UserColumn::Username.eq(username))
            .one(db)
            .await
            .map_err(ErrorInternalServerError)?
            .ok_or(ErrorNotFound(error))?;
    } else if let Some(email) = data.0.email {
        found = user::find()
            .filter(UserColumn::Email.eq(email))
            .one(db)
            .await
            .map_err(ErrorInternalServerError)?
            .ok_or(ErrorNotFound(error))?;
    } else {
        return Err(ErrorBadGateway("Missing username or email"));
    }

    encrypt::verify(&found.password_hash, &data.0.password)?;

    let mut json = json!(found);

    json["token"] = json!(sign(found.id).map_err(ErrorInternalServerError)?);

    Ok(web::Json(json))
}

pub fn routes() -> actix_web::Scope {
    web::scope("/user")
        .service(index)
        .service(store)
        .service(login)
        .service(edit)
        .service(delete)
}
