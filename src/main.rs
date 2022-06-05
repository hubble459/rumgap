#[macro_use]
extern crate rocket;

use rocket::fairing::{self, AdHoc};
use rocket::{Build, Rocket};

use migration::MigratorTrait;
use sea_orm_rocket::Database;

mod pool;
use pool::Db;

pub use entity::post;
pub use entity::post::Entity as Post;
pub mod api;

use api::manga;

async fn run_migrations(rocket: Rocket<Build>) -> fairing::Result {
    let conn = &Db::fetch(&rocket).unwrap().conn;
    let _ = migration::Migrator::up(conn, None).await;
    Ok(rocket)
}

#[launch]
fn rocket() -> _ {
    rocket::build()
        .attach(Db::init())
        .attach(AdHoc::try_on_ignite("Migrations", run_migrations))
        .mount("/api/manga", manga::routes())
}
