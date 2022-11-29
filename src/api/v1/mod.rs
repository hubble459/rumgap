use actix_web::web;

mod data;
mod user;
mod util;

pub fn routes() -> actix_web::Scope {
    web::scope("/v1").service(user::routes())
}
