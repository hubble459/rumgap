use actix_web::error::{ErrorBadRequest, ErrorConflict, ErrorInternalServerError, ErrorNotFound};
use actix_web::http::StatusCode;
use actix_web::{web, HttpResponse, Responder, Result};
use entity::user::{ActiveModel as ActiveUser, Column as UserColumn, Entity as user};
use migration::{Alias, Expr, JoinType};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, IntoActiveModel,
    PaginatorTrait, QueryFilter, QuerySelect, RelationTrait, Set,
};

use crate::api::v1::data;
use crate::api::v1::data::paginate::{Paginate, PaginateQuery};
use crate::api::v1::util::{encrypt, permission, verify};
use crate::middleware::auth::{sign, AuthService};

#[rustfmt::skip]
pub async fn get_user_by_id(db: &DatabaseConnection, user_id: i32) -> Result<data::user::Full> {
    let following_alias = Alias::new("following");
    let followers_alias = Alias::new("followers");

    let found_user = user::find_by_id(user_id)
        .join_as(JoinType::LeftJoin, entity::friend::Relation::User1.def().rev(), following_alias.clone())
        .join_as(JoinType::LeftJoin, entity::friend::Relation::User2.def().rev(), followers_alias.clone())
        .column_as(Expr::col((following_alias, entity::friend::Column::Id)).count(), "count_following")
        .column_as(Expr::col((followers_alias, entity::friend::Column::Id)).count(), "count_followers")
        .group_by(UserColumn::Id)
        .into_model::<data::user::Full>()
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
) -> Result<Paginate<Vec<data::user::Partial>>> {
    let db = conn.as_ref();

    // Create paginate object
    let paginate = user::find()
        .into_model::<data::user::Partial>()
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
    data: web::Json<data::user::Post>,
) -> Result<impl Responder> {
    let new_user = ActiveUser {
        username: Set(verify::username(&data.username)?),
        email: Set(verify::email(&data.email)?),
        password_hash: Set(encrypt::encrypt(&verify::password(&data.password)?)?),
        ..Default::default()
    };

    let db = conn.as_ref();

    let created = new_user.insert(db).await.map_err(|e| {
        if verify::is_conflict(&e) {
            return ErrorConflict("Username or email already in use");
        }
        ErrorInternalServerError(e)
    })?;

    let full_user = get_user_by_id(db, created.id).await?;

    Ok((web::Json(full_user), StatusCode::CREATED))
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
        if verify::is_conflict(&e) {
            return ErrorConflict("Username or email already in use");
        }
        ErrorInternalServerError(e)
    })?;

    let full_user = get_user_by_id(db, created.id).await?;

    Ok(web::Json(full_user))
}

#[get("/{user_id}")]
async fn get(path: web::Path<u16>, conn: web::Data<DatabaseConnection>) -> Result<impl Responder> {
    let user_id = path.into_inner();

    let db = conn.as_ref();

    let full_user = get_user_by_id(db, user_id as i32).await?;

    Ok(web::Json(full_user))
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

    let filter;

    if let Some(username) = data.0.username {
        filter = UserColumn::Username.eq(username.to_ascii_lowercase());
    } else if let Some(email) = data.0.email {
        filter = UserColumn::Email.eq(email.to_ascii_lowercase());
    } else {
        return Err(ErrorBadRequest("Missing username or email"));
    }

    let found = user::find()
        .filter(filter)
        .one(db)
        .await
        .map_err(ErrorInternalServerError)?
        .ok_or(ErrorNotFound(error))?;

    encrypt::verify(&found.password_hash, &data.0.password)?;

    let full_user = get_user_by_id(db, found.id).await?;

    let mut json = json!(full_user);
    json["token"] = json!(sign(full_user.id).map_err(ErrorInternalServerError)?);

    Ok((web::Json(json), StatusCode::CREATED))
}

pub fn routes() -> actix_web::Scope {
    web::scope("/user")
        .service(index)
        .service(store)
        .service(login)
        .service(get)
        .service(edit)
        .service(delete)
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
