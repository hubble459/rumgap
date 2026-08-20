//! Local image-hosting for manga covers (Phase A). Same lazy
//! ensure-downloaded-then-serve pattern as [`crate::util::chapter_images`],
//! but 1:1 per `manga_id` rather than 1:N per chapter, so state lives
//! directly on the `manga` row instead of a separate table.

use std::time::Duration;

use chrono::Utc;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, DatabaseConnection};
use sha2::{Digest, Sha256};
use tonic::Status;

use crate::IMAGE_STORE;

const MAX_ATTEMPT_SESSIONS: i32 = 3;
const RETRY_COOLDOWN_MINUTES: i64 = 5;

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

/// Lazily download (and cache) a manga's cover. Never errors for a
/// *download* failure -- that's represented in the returned model's
/// `cover_status` field so callers (the `/covers/{manga_id}` route) can
/// decide how to degrade, same convention as `ensure_page_downloaded`.
pub async fn ensure_cover_downloaded(
    db: &DatabaseConnection,
    manga: entity::manga::Model,
    referer: &str,
) -> Result<entity::manga::Model, Status> {
    if manga.cover_status == "done" {
        return Ok(manga);
    }

    let Some(source_url) = manga.cover_source_url.clone() else {
        return Ok(manga);
    };

    if manga.cover_status == "failed" {
        if manga.cover_attempts >= MAX_ATTEMPT_SESSIONS {
            return Ok(manga);
        }
        let cooldown_until = manga.updated_at + chrono::Duration::minutes(RETRY_COOLDOWN_MINUTES);
        if Utc::now().naive_utc() < cooldown_until {
            return Ok(manga);
        }
    }

    let mut active: entity::manga::ActiveModel = manga.clone().into();

    match download(&source_url, referer).await {
        Ok((bytes, content_type)) => {
            let ext = ext_for_content_type(&content_type);
            let storage_key = format!("covers/{}.{}", manga.id, ext);
            let checksum = hex::encode(Sha256::digest(&bytes));

            IMAGE_STORE
                .put(&storage_key, bytes)
                .await
                .map_err(|e| Status::internal(format!("Failed to store cover: {e}")))?;

            active.cover_status = Set("done".to_string());
            active.cover_storage_key = Set(Some(storage_key));
            active.cover_content_type = Set(Some(content_type));
            active.cover_checksum = Set(Some(checksum));
        }
        Err(e) => {
            warn!("Failed to download cover for manga {}: {}", manga.id, e);
            active.cover_status = Set("failed".to_string());
            active.cover_attempts = Set(manga.cover_attempts + 1);
        }
    }

    active.update(db).await.map_err(|e| Status::internal(e.to_string()))
}

async fn download(url: &str, referer: &str) -> Result<(Vec<u8>, String), String> {
    let response = manga_parser::HTTP_CLIENT
        .get(url)
        .header("Referer", referer)
        .header("Origin", referer)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let response = response.error_for_status().map_err(|e| e.to_string())?;

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
        .unwrap_or_else(|| "image/jpeg".to_string());

    let bytes = response.bytes().await.map_err(|e| e.to_string())?.to_vec();

    Ok((bytes, content_type))
}
