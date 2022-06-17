#[macro_use]
extern crate rocket;

use rocket::fairing::{self, AdHoc};
use rocket::{Build, Rocket};

use migration::MigratorTrait;
use sea_orm_rocket::Database;

mod api;
mod auth;
mod cors;
mod pool;
use pool::Db;

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
    let _ = migration::Migrator::up(conn, None).await;
    Ok(rocket)
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
}
