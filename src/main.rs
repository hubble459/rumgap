#[macro_use]
extern crate log;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate phf;
#[macro_use]
extern crate serde_json;

use std::env;

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

pub mod proto {
    tonic::include_proto!("rumgap");

    pub(crate) const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("descriptor");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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

    let addr = server_url.parse()?;

    info!("Running server on {}", addr);

    Server::builder()
        .layer(tonic::service::interceptor(move |req| {
            intercept(req, conn.clone())
        }))
        .layer(async_interceptor(interceptor::auth::check_auth))
        .add_service(service::user::server())
        .add_service(service::friend::server())
        .add_service(
            Builder::configure()
                .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
                .build()?,
        )
        .serve(addr)
        .await?;

    Ok(())
}

async fn conn_db(db_url: &str) -> Result<DatabaseConnection, DbErr> {
    // Establish connection to database and apply migrations
    info!("Connecting to database and running migrations...");
    let conn = Database::connect(db_url).await?;
    Migrator::up(&conn, None).await?;
    info!("Connected to the database");

    Ok(conn)
}

fn intercept(mut req: Request<()>, conn: DatabaseConnection) -> Result<Request<()>, Status> {
    println!("Intercepting request: {:?}", req);

    req.extensions_mut().insert(conn);

    Ok(req)
}



macro_rules! export_server {
    ($server:ident, $server_handler:ident) => {
        $crate::export_server!($server, $server_handler, auth = false);
    };
    ($server:ident, $server_handler:ident, auth = true) => {
        pub fn server() -> tonic::service::interceptor::InterceptedService<$server<$server_handler>, $crate::interceptor::auth::LoggedInCheck> {
            $server::with_interceptor($server_handler::default(), $crate::interceptor::auth::logged_in())
        }

        // pub fn server() -> $server<$server_handler> {
        //     if $authorized {
        //         $server::with_interceptor($server_handler::default(), $crate::interceptor::auth::logged_in())
        //     } else {
        //         $server::new($server_handler::default())
        //     }
        // }
    };
    ($server:ident, $server_handler:ident, auth = false) => {
        pub fn server() -> $server<$server_handler> {
            $server::new($server_handler::default())
        }
    };
}

pub(crate) use export_server;
