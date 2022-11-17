#[macro_use]
extern crate rocket;

use rocket::fairing::{self, AdHoc};
use rocket::http::Status;
use rocket::{Build, Rocket};

use migration::MigratorTrait;
use sea_orm_rocket::Database;

mod api;
mod auth;
mod cors;
mod pool;
use pool::Db;

use crate::auth::User;

pub mod pagination;

use api::chapter;
use api::login;
use api::manga;
use api::reading;
use api::register;
use api::search;
use cors::CORS;

async fn run_migrations(rocket: Rocket<Build>) -> fairing::Result {
    let conn = &Db::fetch(&rocket).unwrap().conn;
    let result = migration::Migrator::up(conn, None).await;
    if let Err(e) = result {
        println!("ERROR: {:#?}", e);
        return Err(rocket);
    }
    Ok(rocket)
}

#[get("/auth")]
async fn test_auth(_user: User) -> Status {
    Status::Ok
}

#[launch]
fn rocket() -> _ {
    rocket::build()
        .attach(Db::init())
        .attach(CORS)
        .manage(parser::parser::MangaParser::new())
        .attach(AdHoc::try_on_ignite("Migrations", run_migrations))
        .mount(format!("/api/{}", manga::base()), manga::routes())
        .mount(format!("/api/{}", chapter::base()), chapter::routes())
        .mount("/api", login::routes())
        .mount("/api", register::routes())
        .mount("/api", search::routes())
        .mount(format!("/api/{}", reading::base()), reading::routes())
        .mount("/api", routes![test_auth])
}
