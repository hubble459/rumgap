use once_cell::sync::OnceCell;
use sea_orm::DatabaseConnection;

static CONN: OnceCell<DatabaseConnection> = OnceCell::new();

pub async fn get_db() -> DatabaseConnection {
    if let Some(conn) = CONN.get() {
        return conn.clone();
    } else {
        std::env::set_var("RUST_BACKTRACE", "1");
        dotenvy::dotenv().unwrap();
        log::set_max_level(log::LevelFilter::Info);

        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL is not set in .env file");
        let conn = crate::conn_db(&db_url).await.unwrap();
        CONN.set(conn.clone()).unwrap();

        conn.clone()
    }
}

macro_rules! test_resource {
    (
        $fn_name:ident $base:literal;
        $(
            $(#[$meta:meta])*
            $method:ident: $path:literal $($index:literal $($field:ident)?)* $(=> $status:expr)? $(; $data:expr)? $(, $header_key:ident: $($header_value1:literal $($header_value2:ident)?)+)*;;
        )*
     ) => {
        #[allow(unused_mut, path_statements)]
        #[actix_web::test]
        async fn $fn_name() {
            use actix_web::test::TestRequest;
            use actix_web::{test, App, web};
            use actix_web::http::StatusCode;
            use actix_web::http::header::*;
            use actix_web::body::{BodySize, MessageBody};

            let app = test::init_service(
                App::new()
                    .app_data(web::Data::new(crate::test::get_db().await))
                    .configure(crate::init_routes),
            )
            .await;

            let mut responses: Vec<serde_json::value::Value> = vec![];

            $(
                let uri = concat!($base, $path).to_string();
                let mut extras = String::new();
                $(extras += { $index $(;responses[$index][stringify!($field)].to_string().trim_matches('"'))? };)*
                let uri = (uri + &extras);
                let uri = uri.trim_end_matches("/");
                let resp = TestRequest::$method()
                    .uri(uri)
                    $(.set_json($data))?
                    $(.append_header(($header_key, {
                        let mut value = String::new();
                        $(
                            value += { $header_value1 $(;responses[$header_value1][stringify!($header_value2)].to_string().trim_matches('"'))? };
                        )+
                        value
                    })))*
                    .send_request(&app)
                    .await;

                info!("[REQUEST] {:?}", resp.request());
                info!("[RESPONSE] {:?}", &resp);

                assert_eq!(resp.status(), { StatusCode::OK $(;$status)? });

                let data: serde_json::value::Value;
                match resp.response().body().size() {
                    BodySize::Sized(size) if size > 0 => {
                        data = test::read_body_json(resp).await;
                    }
                    _ => {
                        data = json!({});
                    }
                }

                responses.push(data);
            )*
        }
    }
}

pub(crate) use test_resource;