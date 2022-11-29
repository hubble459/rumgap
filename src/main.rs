#[macro_use]
extern crate log;
#[macro_use]
extern crate actix_web;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate bitflags;

mod api;
mod middleware;
#[cfg(test)]
mod test;

use std::env;
use std::time::Duration;

use actix_files::Files as Fs;
use actix_web::middleware::{Logger, NormalizePath};
use actix_web::{web, App, HttpServer};
use listenfd::ListenFd;
use migration::{DbErr, Migrator, MigratorTrait};
use sea_orm::{Database, DatabaseConnection};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "info");
    std::env::set_var("RUST_BACKTRACE", "1");
    env_logger::init();

    // Get env vars
    dotenvy::dotenv().ok();
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL is not set in .env file");
    let host = env::var("HOST").unwrap_or(String::from("127.0.0.1"));
    let port = env::var("PORT").unwrap_or(String::from("8000"));
    let server_url = format!("{}:{}", host, port);

    // Establish connection to database and apply migrations
    let conn = conn_db(&db_url).await.unwrap();

    // create server and try to serve over socket if possible
    let mut listen_fd = ListenFd::from_env();
    let mut server = HttpServer::new(move || {
        App::new()
            .service(Fs::new("/static", "./static"))
            .app_data(web::Data::new(conn.clone()))
            .wrap(Logger::default())
            .wrap(NormalizePath::new(actix_web::middleware::TrailingSlash::Always))
            // .default_service(web::route().to(not_found))
            .configure(init_routes)
    })
    .keep_alive(Duration::from_secs(75));

    server = match listen_fd.take_tcp_listener(0)? {
        Some(listener) => server.listen(listener)?,
        None => server.bind(&server_url)?,
    };

    info!("Starting server at {}", server_url);

    server.run().await
}

async fn conn_db(db_url: &str) -> Result<DatabaseConnection, DbErr> {
    // Establish connection to database and apply migrations
    info!("Connecting to database and running migrations...");
    let conn = Database::connect(db_url).await?;
    Migrator::up(&conn, None).await?;
    info!("Done");

    Ok(conn)
}

fn init_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(api::routes());
}
