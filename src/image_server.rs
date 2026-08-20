//! Plain HTTP server for serving downloaded chapter page images.
//!
//! Not gRPC -- protobuf framing isn't how a Flutter `Image`/browser `<img>`
//! consumes bytes. Runs on its own port (`IMAGE_PORT`, default 8001) since
//! `tonic::transport::Server` wants exclusive control of its listener.

use axum::extract::{Path, Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::util::chapter_images::{ensure_chapter_image_rows, ensure_page_downloaded};
use crate::util::cover_images::ensure_cover_downloaded;
use crate::util::scraper_hostnames::is_allowed_hostname;
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
        .route("/covers/{manga_id}", get(get_cover))
        .route("/proxy", get(get_proxy))
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
            return serve_from_store(storage_key, content_type, checksum).await;
        }
    }

    // Durably failed (or still cooling down between attempt-sessions):
    // degrade to exactly today's raw-hotlink behavior for this one page
    // only, rather than a broken image. Every other successfully-downloaded
    // page in the same chapter keeps being served locally.
    Redirect::temporary(&row.source_url).into_response()
}

/// `GET /covers/{manga_id}` -- same lazy pattern as `get_image`, but 1:1 per
/// manga rather than per-page. Referer for the download is the manga's
/// primary source URL, matching the rule that only the primary source's
/// scrape ever populates canonical cover data.
async fn get_cover(State(db): State<DatabaseConnection>, Path(manga_id): Path<i32>, headers: HeaderMap) -> Response {
    let manga = match entity::manga::Entity::find_by_id(manga_id).one(&db).await {
        Ok(Some(manga)) => manga,
        Ok(None) => return (StatusCode::NOT_FOUND, "Manga not found").into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let Some(source_url) = manga.cover_source_url.clone() else {
        return (StatusCode::NOT_FOUND, "This manga has no cover").into_response();
    };

    if manga.cover_status == "done" {
        if let Some(checksum) = &manga.cover_checksum {
            if let Some(if_none_match) = headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
                if if_none_match.trim_matches('"') == checksum {
                    return StatusCode::NOT_MODIFIED.into_response();
                }
            }
        }
    }

    let primary_source = entity::manga_source::Entity::find()
        .filter(entity::manga_source::Column::MangaId.eq(manga_id))
        .filter(entity::manga_source::Column::IsPrimary.eq(true))
        .one(&db)
        .await;
    let referer = match primary_source {
        Ok(Some(source)) => source.url,
        _ => source_url.clone(),
    };

    let manga = match ensure_cover_downloaded(&db, manga, &referer).await {
        Ok(manga) => manga,
        Err(status) => return status_to_response(status),
    };

    if manga.cover_status == "done" {
        if let (Some(storage_key), Some(content_type), Some(checksum)) = (
            &manga.cover_storage_key,
            &manga.cover_content_type,
            &manga.cover_checksum,
        ) {
            return serve_from_store(storage_key, content_type, checksum).await;
        }
    }

    // Durably failed / cooling down: degrade to the raw source URL for this
    // one manga's cover, same as chapter images do for a failed page.
    Redirect::temporary(&source_url).into_response()
}

#[derive(Deserialize)]
struct ProxyQuery {
    url: String,
    referer: String,
}

/// `GET /proxy?url=...&referer=...` -- transient, hash-keyed cache for
/// ephemeral search-result covers (Phase A2). No DB row: the extension is
/// unknown until the first real download, so a cache lookup probes each
/// candidate extension against `ImageStore` directly rather than tracking
/// content_type separately.
///
/// Hosts for both `url` and `referer` MUST be on the scraper hostname
/// allowlist (`crate::util::scraper_hostnames`) or this would be an open
/// SSRF proxy -- reachable by anyone who finds the endpoint, not just wuxia,
/// since it's unauthenticated and potentially internet-facing.
async fn get_proxy(Query(query): Query<ProxyQuery>) -> Response {
    let Ok(target) = manga_parser::Url::parse(&query.url) else {
        return (StatusCode::BAD_REQUEST, "Invalid url").into_response();
    };
    let Ok(referer) = manga_parser::Url::parse(&query.referer) else {
        return (StatusCode::BAD_REQUEST, "Invalid referer").into_response();
    };

    let target_host = target.host_str().unwrap_or_default().to_lowercase();
    let referer_host = referer.host_str().unwrap_or_default().to_lowercase();
    if !is_allowed_hostname(&target_host) || !is_allowed_hostname(&referer_host) {
        warn!(
            "[Proxy] Rejected non-allowlisted host: url_host={}, referer_host={}",
            target_host, referer_host
        );
        return (
            StatusCode::BAD_REQUEST,
            "Host not recognized as a supported manga source",
        )
            .into_response();
    }

    let hash = hex::encode(Sha256::digest(query.url.as_bytes()));

    if let Some((bytes, content_type)) = find_cached_proxy(&hash).await {
        return respond_with_bytes(bytes, &content_type);
    }

    let response = match manga_parser::HTTP_CLIENT
        .get(target)
        .header("Referer", query.referer.clone())
        .header("Origin", query.referer)
        .send()
        .await
    {
        Ok(response) => response,
        Err(e) => return (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    };
    let response = match response.error_for_status() {
        Ok(response) => response,
        Err(e) => return (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    };

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
        .unwrap_or_else(|| "image/jpeg".to_string());

    let bytes = match response.bytes().await {
        Ok(bytes) => bytes.to_vec(),
        Err(e) => return (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    };

    let ext = ext_for_content_type(&content_type);
    let key = format!("proxy/{hash}.{ext}");
    if let Err(e) = IMAGE_STORE.put(&key, bytes.clone()).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to cache proxied image: {e}"),
        )
            .into_response();
    }

    respond_with_bytes(bytes, &content_type)
}

const PROXY_EXTENSIONS: &[&str] = &["jpg", "png", "webp", "gif", "avif", "bmp"];

async fn find_cached_proxy(hash: &str) -> Option<(Vec<u8>, String)> {
    for ext in PROXY_EXTENSIONS {
        let key = format!("proxy/{hash}.{ext}");
        if IMAGE_STORE.exists(&key).await {
            if let Ok(bytes) = IMAGE_STORE.get(&key).await {
                return Some((bytes, content_type_for_ext(ext)));
            }
        }
    }
    None
}

fn content_type_for_ext(ext: &str) -> String {
    match ext {
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        _ => "image/jpeg",
    }
    .to_string()
}

fn ext_for_content_type(content_type: &str) -> &'static str {
    match content_type {
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/avif" => "avif",
        "image/bmp" => "bmp",
        _ => "jpg",
    }
}

fn respond_with_bytes(bytes: Vec<u8>, content_type: &str) -> Response {
    let checksum = hex::encode(Sha256::digest(&bytes));
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

async fn serve_from_store(storage_key: &str, content_type: &str, checksum: &str) -> Response {
    match IMAGE_STORE.get(storage_key).await {
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read stored image: {e}"),
        )
            .into_response(),
    }
}

fn status_to_response(status: tonic::Status) -> Response {
    let code = match status.code() {
        tonic::Code::NotFound => StatusCode::NOT_FOUND,
        tonic::Code::InvalidArgument => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (code, status.message().to_string()).into_response()
}
