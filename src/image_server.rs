//! Plain HTTP server for serving downloaded chapter page images.
//!
//! Not gRPC -- protobuf framing isn't how a Flutter `Image`/browser `<img>`
//! consumes bytes. Runs on its own port (`IMAGE_PORT`, default 8001) since
//! `tonic::transport::Server` wants exclusive control of its listener.

use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use sea_orm::{DatabaseConnection, EntityTrait};

use crate::util::chapter_images::{ensure_chapter_image_rows, ensure_page_downloaded};
use crate::IMAGE_STORE;

/// Start serving `GET /images/{chapter_id}/{page_index}` on `IMAGE_PORT`
/// (default 8001). Intended to be `tokio::spawn`'d alongside the updater
/// loop in `main.rs`.
pub async fn serve(db: DatabaseConnection) {
    let host = std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port: u16 = std::env::var("IMAGE_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8001);
    let addr = format!("{host}:{port}");

    let app = Router::new()
        .route("/images/{chapter_id}/{page_index}", get(get_image))
        .with_state(db);

    info!("Running image server on {}", addr);

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(e) => {
            error!("Failed to bind image server to {}: {}", addr, e);
            return;
        }
    };

    if let Err(e) = axum::serve(listener, app).await {
        error!("Image server crashed: {}", e);
    }
}

async fn get_image(
    State(db): State<DatabaseConnection>,
    Path((chapter_id, page_index)): Path<(i32, i32)>,
    headers: HeaderMap,
) -> Response {
    let chapter = match entity::chapter::Entity::find_by_id(chapter_id).one(&db).await {
        Ok(Some(chapter)) => chapter,
        Ok(None) => return (StatusCode::NOT_FOUND, "Chapter not found").into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    // Ensures rows exist even if this URL is hit before the owning
    // Chapter.Images gRPC call ever ran (e.g. a stale/bookmarked URL).
    let rows = match ensure_chapter_image_rows(&db, &chapter).await {
        Ok(rows) => rows,
        Err(status) => return status_to_response(status),
    };

    let Some(row) = rows.into_iter().find(|row| row.page_index == page_index) else {
        return (StatusCode::NOT_FOUND, "Page not found").into_response();
    };

    // Cheap conditional-GET short circuit before touching disk: these files
    // never change once downloaded, so a matching If-None-Match means the
    // client already has the exact bytes.
    if row.status == "done" {
        if let Some(checksum) = &row.checksum {
            if let Some(if_none_match) = headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
                if if_none_match.trim_matches('"') == checksum {
                    return StatusCode::NOT_MODIFIED.into_response();
                }
            }
        }
    }

    // Blocks only on the 1-3 pages actually being viewed, never a whole
    // chapter -- the per-page lock + global semaphore live inside this call.
    let row = match ensure_page_downloaded(&db, &chapter, row).await {
        Ok(row) => row,
        Err(status) => return status_to_response(status),
    };

    if row.status == "done" {
        if let (Some(storage_key), Some(content_type), Some(checksum)) =
            (&row.storage_key, &row.content_type, &row.checksum)
        {
            return match IMAGE_STORE.get(storage_key).await {
                Ok(bytes) => {
                    let mut response_headers = HeaderMap::new();
                    if let Ok(value) = content_type.parse() {
                        response_headers.insert(CONTENT_TYPE, value);
                    }
                    response_headers.insert(CACHE_CONTROL, "public, max-age=31536000, immutable".parse().unwrap());
                    if let Ok(value) = format!("\"{checksum}\"").parse() {
                        response_headers.insert(ETAG, value);
                    }
                    (StatusCode::OK, response_headers, bytes).into_response()
                }
                Err(e) => {
                    (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read stored image: {e}")).into_response()
                }
            };
        }
    }

    // Durably failed (or still cooling down between attempt-sessions):
    // degrade to exactly today's raw-hotlink behavior for this one page
    // only, rather than a broken image. Every other successfully-downloaded
    // page in the same chapter keeps being served locally.
    Redirect::temporary(&row.source_url).into_response()
}

fn status_to_response(status: tonic::Status) -> Response {
    let code = match status.code() {
        tonic::Code::NotFound => StatusCode::NOT_FOUND,
        tonic::Code::InvalidArgument => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (code, status.message().to_string()).into_response()
}
