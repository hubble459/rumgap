#[macro_use]
extern crate log;
#[macro_use]
extern crate actix_web;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate phf;
#[macro_use(json)]
extern crate serde_json;

mod api;
mod middleware;
#[cfg(test)]
mod test;

use std::env;
use std::time::Duration;

use actix_files::Files as Fs;
use actix_web::body::{BoxBody, MessageBody};
use actix_web::dev::Service;
use actix_web::http::header::{self, HeaderValue};
use actix_web::middleware::{Compress, Logger, NormalizePath};
use actix_web::{web, App, HttpServer, ResponseError};
use derive_more::{Display, Error};
use futures::FutureExt;
use listenfd::ListenFd;
use migration::{DbErr, Migrator, MigratorTrait};
use sea_orm::{Database, DatabaseConnection};

#[derive(Debug, Error, Display)]
#[display(fmt = "owo")]
struct JsonError(pub actix_web::Error);

impl ResponseError for JsonError {
    fn error_response(&self) -> actix_web::HttpResponse<actix_web::body::BoxBody> {
        let error = &self.0;
        let mut response = error.error_response();
        let headers = response.headers_mut();
        headers.append(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        let status = response.status();
        response.set_body(BoxBody::new(
            json!({
                "error": error.to_string(),
                "code": status.to_string(),
            })
            .to_string(),
        ))
    }

    fn status_code(&self) -> actix_web::http::StatusCode {
        self.0.error_response().status()
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_BACKTRACE", "1");
    log4rs::init_file("log4rs.yml", Default::default()).unwrap();

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
            .wrap_fn(|req, srv| {
                srv.call(req).map(|res| {
                    res.map(|mut res| {
                        if !res.status().is_success() {
                            let headers = res.headers_mut();
                            headers.append(
                                header::CONTENT_TYPE,
                                HeaderValue::from_static("application/json"),
                            );
                            let cloned_res = res.request().clone();
                            return res.map_body(|head, body| {
                                let bytes = body.try_into_bytes().unwrap();
                                let mut message = String::from_utf8(bytes.to_vec()).unwrap();
                                if message.is_empty() {
                                    message = head.reason().to_string();
                                }
                                let code = head.status.as_u16();
                                if code == 500 {
                                    error!("{} {:?}", cloned_res.uri().to_string(), message);
                                }
                                BoxBody::new(
                                    json!({
                                        "error": message,
                                        "code": code,
                                    })
                                    .to_string(),
                                )
                            });
                        }
                        res
                    })
                })
            })
            .service(Fs::new("/static", "./static"))
            .app_data(web::Data::new(conn.clone()))
            .wrap(Logger::new("%{r}a %r %s %T").log_target("http_log"))
            .wrap(Compress::default())
            .wrap(NormalizePath::new(
                actix_web::middleware::TrailingSlash::Trim,
            ))
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
    info!("Connected to the database");

    Ok(conn)
}

fn init_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(api::routes());
}
