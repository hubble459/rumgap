use actix_web::test::TestRequest;
use actix_web::{http, test, App};
use once_cell::sync::OnceCell;

use crate::*;

static CONN: OnceCell<DatabaseConnection> = OnceCell::new();

async fn get_db() -> DatabaseConnection {
    if let Some(conn) = CONN.get() {
        return conn.clone();
    } else {
        std::env::set_var("RUST_LOG", "info");
        std::env::set_var("RUST_BACKTRACE", "1");
        dotenvy::dotenv().unwrap();
        env_logger::init();

        let db_url = env::var("DATABASE_URL").expect("DATABASE_URL is not set in .env file");
        let conn = conn_db(&db_url).await.unwrap();
        CONN.set(conn.clone()).unwrap();

        conn.clone()
    }
}

#[actix_web::test]
async fn test_login() {
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(get_db().await))
            .configure(init_routes),
    )
    .await;

    let resp = TestRequest::post()
        .uri("/api/v1/user")
        .set_json(json!({
            "username": "test",
            "password": "test"
        }))
        .append_header((header::AUTHORIZATION, "owo"))
        .send_request(&app)
        .await;

    assert_eq!(resp.status(), http::StatusCode::OK);

    info!("{:?}", resp.request());

    let data: serde_json::value::Value = test::read_body_json(resp).await;
    data["owo"].as_str().unwrap();
}
