#[macro_use]
extern crate log;

#[macro_use]
extern crate actix_web;

mod api;
mod middleware;
use actix_web::{error, middleware::Logger, web, App, Error, HttpServer, Result};
use migration::{Migrator, MigratorTrait};
use sea_orm::{Database, DatabaseConnection};

#[get("/")]
async fn index(data: web::Data<DatabaseConnection>) -> Result<&'static str, Error> {
    let conn = &data;

    Ok("fuck")
}

#[post("/login")]
async fn login(pool: web::Data<DatabaseConnection>) -> Result<&'static str, Error> {
    Ok("owo")
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    use actix_files::Files as Fs;
    use listenfd::ListenFd;
    use std::{env, time::Duration};

    std::env::set_var("RUST_LOG", "info");
    std::env::set_var("RUST_BACKTRACE", "1");
    env_logger::init();

    // get env vars
    dotenvy::dotenv().ok();
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL is not set in .env file");
    let host = env::var("HOST").unwrap_or(String::from("127.0.0.1"));
    let port = env::var("PORT").unwrap_or(String::from("8000"));
    let server_url = format!("{}:{}", host, port);

    // establish connection to database and apply migrations
    // -> create post table if not exists
    let conn = Database::connect(&db_url).await.unwrap();
    Migrator::up(&conn, None).await.unwrap();

    // create server and try to serve over socket if possible
    let mut listen_fd = ListenFd::from_env();
    let mut server = HttpServer::new(move || {
        App::new()
            .service(Fs::new("/static", "./api/static"))
            // .app_data(web::Data::new(state.clone()))
            .app_data(web::Data::new(conn.clone()))
            .wrap(Logger::default()) // enable logger
            // .default_service(web::route().to(not_found))
            .configure(init)
    })
    .keep_alive(Duration::from_secs(75));

    server = match listen_fd.take_tcp_listener(0)? {
        Some(listener) => server.listen(listener)?,
        None => server.bind(&server_url)?,
    };

    println!("Starting server at {}", server_url);

    server.run().await
}

fn init(cfg: &mut web::ServiceConfig) {
    cfg.service(index);
    // cfg.service(new);
    // cfg.service(create);
    // cfg.service(edit);
    // cfg.service(update);
    // cfg.service(delete);
}
