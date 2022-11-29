use actix_web::web;

mod data;
mod route;
#[cfg(test)]
mod test;
mod util;

pub fn routes() -> actix_web::Scope {
    web::scope("/v1").service(route::user::routes())
}
