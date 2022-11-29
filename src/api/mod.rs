use actix_web::{Scope, web};

mod v1;

pub fn routes() -> Scope {
    web::scope("/api").service(v1::routes())
}