#[macro_use]
extern crate log;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate phf;

use std::env;

use crate::util::auth::Authorize;
use hyper::Uri;
use manga_parser::scraper::scraper_manager::ScraperManager;
use migration::{DbErr, Migrator, MigratorTrait};
use sea_orm::{Database, DatabaseConnection};
use tonic::transport::Server;
use tonic::{Request, Status};
use tonic_async_interceptor::async_interceptor;
use tonic_reflection::server::Builder;

mod data;
mod interceptor;
mod service;
mod util;

lazy_static! {
    static ref MANGA_PARSER: ScraperManager = manga_parser::scraper::scraper_manager::ScraperManager::default();
}

/// Load all ProtoBuf files
pub mod proto {
    tonic::include_proto!("rumgap.v1");

    pub(crate) const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("descriptor");
}

/// Start the server
///
/// Init log4rs
/// Init database
/// Add all services
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    std::env::set_var("RUST_BACKTRACE", "1");
    log4rs::init_file("log4rs.yml", Default::default()).ok();

    // Get env vars
    dotenvy::dotenv().ok();
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL is not set in .env file");
    let host = env::var("HOST").unwrap_or(String::from("127.0.0.1"));
    let port = env::var("PORT").unwrap_or(String::from("8000"));
    let server_url = format!("{host}:{port}");
    let addr = server_url.parse()?;

    // Establish connection to database and apply migrations
    let conn = conn_db(&db_url).await.unwrap();

    info!("Running server on {}", addr);

    // Start updater
    let cloned_conn = conn.clone();
    tokio::spawn(async move {
        crate::util::updater::watch_updates(&cloned_conn).await;
    });

    Server::builder()
        .layer(tonic::service::interceptor(move |req| inject_db(req, conn.clone())))
        .layer(async_interceptor(interceptor::auth::check_auth))
        // .layer(tower::ServiceBuilder::new().layer_fn(Logger::new))
        .layer(tonic::service::interceptor(logger))
        .add_service(service::v1::user::server())
        .add_service(service::v1::friend::server())
        .add_service(service::v1::manga::server())
        .add_service(service::v1::chapter::server())
        .add_service(service::v1::reading::server())
        .add_service(service::v1::search::server())
        .add_service(service::v1::meta::server())
        .add_service(
            Builder::configure()
                .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
                .build_v1()?,
        )
        .serve(addr)
        .await?;

    Ok(())
}

/// Make a connection to the database and run migrations
async fn conn_db(db_url: &str) -> Result<DatabaseConnection, DbErr> {
    // Establish connection to database and apply migrations
    info!("Connecting to database and running migrations...");
    let conn = Database::connect(db_url).await?;
    Migrator::up(&conn, None).await?;
    info!("Connected to the database");

    Ok(conn)
}

/// Add the database to all requests via their extensions
fn inject_db(mut req: Request<()>, conn: DatabaseConnection) -> Result<Request<()>, Status> {
    req.extensions_mut().insert(conn);

    Ok(req)
}

/// Log the incoming request
fn logger(req: Request<()>) -> Result<Request<()>, Status> {
    let logged_in = req.authorize().ok();
    let target_uri = req.extensions().get::<Uri>();

    info!(
        "[{}] -> [{:?}] ({})",
        req.remote_addr().map_or(String::from("unknown"), |ip| ip.to_string()),
        target_uri.map_or(String::from("unknown"), |uri| uri.path().to_string()),
        logged_in.map_or("#anon#".to_string(), |user| user.username.clone()),
    );

    Ok(req)
}

/// Macro for exporting a service
macro_rules! export_service {
    ($server:ident, $server_handler:ident) => {
        pub fn server() -> $server<$server_handler> {
            $server::new($server_handler::default())
                .send_compressed(tonic::codec::CompressionEncoding::Gzip)
                .accept_compressed(tonic::codec::CompressionEncoding::Gzip)
        }
    };
    ($server:ident, $server_handler:ident, auth = $auth:expr) => {
        pub fn server() -> tonic::service::interceptor::InterceptedService<
            $server<$server_handler>,
            $crate::interceptor::auth::LoggedInCheck,
        > {
            tower::ServiceBuilder::new()
                .layer(tonic::service::interceptor(
                    crate::interceptor::auth::LoggedInCheck::new(UserPermissions::USER),
                ))
                .service(
                    $server::new($server_handler::default())
                        .send_compressed(tonic::codec::CompressionEncoding::Gzip)
                        .accept_compressed(tonic::codec::CompressionEncoding::Gzip),
                )
        }
    };
}

pub(crate) use export_service;
