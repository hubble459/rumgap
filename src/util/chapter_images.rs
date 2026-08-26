//! Local image-hosting download pipeline.
//!
//! Two entry points:
//! - [`ensure_chapter_image_rows`]: scrape the page list for a chapter via
//!   `MANGA_PARSER.chapter_images` **exactly once, ever**, and make sure a
//!   `chapter_image` row exists per page. This is what fixes today's
//!   "re-scrapes on every gRPC call" behavior, as a side effect of caching
//!   rather than a separate change.
//! - [`ensure_page_downloaded`]: lazily download (and cache) a single page,
//!   de-duplicated across concurrent callers via a per-page lock and bounded
//!   by a process-global semaphore.
//!
//! [`prefetch_chapter`] and [`refresh_chapter_images`] are built on top of
//! these two for the eager-prefetch/backfill and force-refresh use cases
//! respectively.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures::stream::{self, StreamExt};
use manga_parser::scraper::MangaScraper;
use manga_parser::Url;
use migration::OnConflict;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};
use tonic::Status;

use crate::{IMAGE_STORE, MANGA_PARSER};

/// Sites known to reject a Referer that points at the chapter page itself,
/// requiring the image's own URL as Referer instead. Faithfully replicated
/// from wuxia's `lib/util/tools.dart` `getReferer()` (`mangaCoverReferer`).
const SELF_REFERER_HOSTNAMES: &[&str] = &["manhuagui.com"];

/// How many attempt-sessions (each already internally retried/backed-off by
/// `manga_parser::HTTP_CLIENT`'s own middleware) a page gets before it's
/// considered durably failed until a manual `RefreshImages`.
const MAX_ATTEMPT_SESSIONS: i32 = 3;

/// Cooldown between attempt-sessions for a page that's currently `failed`.
const RETRY_COOLDOWN_MINUTES: i64 = 5;

type PageKey = (i32, i32);
type PageLockMap = std::sync::Mutex<HashMap<PageKey, Arc<AsyncMutex<()>>>>;

lazy_static! {
    /// Per-`(chapter_id, page_index)` locks so concurrent requests for the
    /// same page converge on one download instead of duplicating work.
    static ref PAGE_LOCKS: PageLockMap = std::sync::Mutex::new(HashMap::new());
    /// Process-global bound on how many images download *at once*, shared by
    /// the lazy on-demand path, eager prefetch, and backfill alike.
    static ref DOWNLOAD_SEMAPHORE: Semaphore = Semaphore::new(download_concurrency());
}

fn download_concurrency() -> usize {
    std::env::var("CHAPTER_IMAGE_DOWNLOAD_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(4)
}

fn page_lock(chapter_id: i32, page_index: i32) -> Arc<AsyncMutex<()>> {
    let mut locks = PAGE_LOCKS.lock().unwrap();
    locks
        .entry((chapter_id, page_index))
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

/// Ensure `chapter_image` rows exist for a chapter, returning them
/// (ordered by `page_index`). If rows already exist, they're returned as-is
/// without ever re-scraping. Otherwise `MANGA_PARSER.chapter_images` is
/// called exactly once and one `pending` row is inserted per page.
pub async fn ensure_chapter_image_rows(
    db: &DatabaseConnection,
    chapter: &entity::chapter::Model,
) -> Result<Vec<entity::chapter_image::Model>, Status> {
    let existing = find_rows(db, chapter.id).await?;
    if !existing.is_empty() {
        return Ok(existing);
    }

    let url = Url::parse(&chapter.url).map_err(|e| Status::invalid_argument(e.to_string()))?;
    let images = crate::util::scrape_log::record(
        db,
        "chapter_images",
        &url,
        None,
        Some(chapter.manga_source_id),
        MANGA_PARSER.chapter_images(&url),
    )
    .await?;

    debug!(
        "Scraped {} image(s) for chapter {} [{}]",
        images.len(),
        chapter.id,
        chapter.url
    );

    if images.is_empty() {
        return Ok(vec![]);
    }

    let rows: Vec<entity::chapter_image::ActiveModel> = images
        .iter()
        .enumerate()
        .map(|(page_index, image_url)| entity::chapter_image::ActiveModel {
            chapter_id: Set(chapter.id),
            page_index: Set(page_index as i32),
            source_url: Set(image_url.to_string()),
            status: Set("pending".to_string()),
            ..Default::default()
        })
        .collect();

    entity::chapter_image::Entity::insert_many(rows)
        .on_conflict(
            OnConflict::columns([
                entity::chapter_image::Column::ChapterId,
                entity::chapter_image::Column::PageIndex,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(db)
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

    find_rows(db, chapter.id).await
}

async fn find_rows(db: &DatabaseConnection, chapter_id: i32) -> Result<Vec<entity::chapter_image::Model>, Status> {
    entity::chapter_image::Entity::find()
        .filter(entity::chapter_image::Column::ChapterId.eq(chapter_id))
        .order_by_asc(entity::chapter_image::Column::PageIndex)
        .all(db)
        .await
        .map_err(|e| Status::internal(e.to_string()))
}

/// Lazily download (and cache) a single page, blocking only on the page
/// itself. Concurrent/duplicate calls for the same page converge via a
/// per-page lock; overall throughput is bounded by
/// `CHAPTER_IMAGE_DOWNLOAD_CONCURRENCY`.
///
/// Never returns an error for a *download* failure -- that's represented in
/// the returned row's `status` field (`failed`) so callers (the image HTTP
/// server, prefetch, backfill) can each decide how to degrade. Errors here
/// are only for infra problems (DB unreachable, etc).
pub async fn ensure_page_downloaded(
    db: &DatabaseConnection,
    chapter: &entity::chapter::Model,
    row: entity::chapter_image::Model,
) -> Result<entity::chapter_image::Model, Status> {
    if row.status == "done" {
        return Ok(row);
    }

    let lock = page_lock(row.chapter_id, row.page_index);
    let _guard = lock.lock().await;

    // Re-fetch inside the lock: another waiter may have already finished
    // this exact page while we were waiting for the mutex.
    let row = entity::chapter_image::Entity::find_by_id((row.chapter_id, row.page_index))
        .one(db)
        .await
        .map_err(|e| Status::internal(e.to_string()))?
        .ok_or_else(|| Status::not_found("chapter_image row disappeared"))?;

    if row.status == "done" {
        return Ok(row);
    }

    if row.status == "failed" {
        if row.attempts >= MAX_ATTEMPT_SESSIONS {
            // Durably failed -- no more retries until a manual RefreshImages.
            return Ok(row);
        }
        let cooldown_until = row.updated_at + chrono::Duration::minutes(RETRY_COOLDOWN_MINUTES);
        if Utc::now().naive_utc() < cooldown_until {
            // Still cooling down since the last attempt-session.
            return Ok(row);
        }
    }

    let _permit = DOWNLOAD_SEMAPHORE
        .acquire()
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

    let mut active: entity::chapter_image::ActiveModel = row.clone().into();
    match download_page(chapter, &row).await {
        Ok((bytes, content_type)) => {
            let ext = ext_for_content_type(&content_type);
            let storage_key = format!("{}/{}.{}", row.chapter_id, row.page_index, ext);
            let checksum = hex::encode(Sha256::digest(&bytes));
            let byte_size = bytes.len() as i64;
            let dimensions = crate::util::image_dimensions::read_dimensions(&bytes);

            IMAGE_STORE
                .put(&storage_key, bytes)
                .await
                .map_err(|e| Status::internal(format!("Failed to store image: {e}")))?;

            active.status = Set("done".to_string());
            active.storage_key = Set(Some(storage_key));
            active.content_type = Set(Some(content_type));
            active.byte_size = Set(Some(byte_size));
            active.checksum = Set(Some(checksum));
            active.error = Set(None);
            active.width = Set(dimensions.map(|(w, _)| w as i32));
            active.height = Set(dimensions.map(|(_, h)| h as i32));
        }
        Err(e) => {
            warn!(
                "Failed to download chapter {} page {}: {}",
                row.chapter_id, row.page_index, e
            );
            active.status = Set("failed".to_string());
            active.attempts = Set(row.attempts + 1);
            active.error = Set(Some(e));
        }
    }

    active.update(db).await.map_err(|e| Status::internal(e.to_string()))
}

/// Build the Referer/Origin used for a page download, replicating wuxia's
/// `getReferer()` special case: manhuagui.com's CDN rejects a Referer
/// pointing at the chapter page, and instead wants the image's own URL.
fn referer_for(chapter_url: &str, image_url: &str) -> String {
    if SELF_REFERER_HOSTNAMES
        .iter()
        .any(|hostname| chapter_url.contains(hostname))
    {
        image_url.to_string()
    } else {
        chapter_url.to_string()
    }
}

async fn download_page(
    chapter: &entity::chapter::Model,
    row: &entity::chapter_image::Model,
) -> Result<(Vec<u8>, String), String> {
    let referer = referer_for(&chapter.url, &row.source_url);

    let response = manga_parser::HTTP_CLIENT
        .get(row.source_url.clone())
        .header("Referer", referer.clone())
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

/// Fire-and-forget-friendly: download every page of a chapter, respecting the
/// same lock/semaphore as the lazy on-demand path. Used by the eager
/// prefetch hook (new chapters for actively-read manga) and by
/// `BackfillImages` (one chapter at a time, from the walker).
pub async fn prefetch_chapter(db: DatabaseConnection, chapter: entity::chapter::Model) {
    let rows = match ensure_chapter_image_rows(&db, &chapter).await {
        Ok(rows) => rows,
        Err(e) => {
            error!("[Prefetch] Failed to list images for chapter {}: {:#?}", chapter.id, e);
            return;
        }
    };

    let concurrency = download_concurrency();
    stream::iter(rows)
        .for_each_concurrent(concurrency, |row| {
            let db = &db;
            let chapter = &chapter;
            async move {
                if let Err(e) = ensure_page_downloaded(db, chapter, row).await {
                    error!(
                        "[Prefetch] Failed to download page for chapter {}: {:#?}",
                        chapter.id, e
                    );
                }
            }
        })
        .await;
}

/// Force-refresh (manual only, `Chapter.RefreshImages`): delete a chapter's
/// existing `chapter_image` rows -- and their stored bytes -- then re-scrape
/// and re-download from scratch. For the handful of sites that briefly serve
/// a placeholder/troll image for freshly-scraped chapters.
pub async fn refresh_chapter_images(
    db: &DatabaseConnection,
    chapter: &entity::chapter::Model,
) -> Result<Vec<entity::chapter_image::Model>, Status> {
    let existing = find_rows(db, chapter.id).await?;

    for row in &existing {
        if let Some(storage_key) = &row.storage_key {
            if let Err(e) = IMAGE_STORE.delete(storage_key).await {
                warn!("Failed to delete stored image {}: {}", storage_key, e);
            }
        }
    }

    entity::chapter_image::Entity::delete_many()
        .filter(entity::chapter_image::Column::ChapterId.eq(chapter.id))
        .exec(db)
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

    ensure_chapter_image_rows(db, chapter).await
}

#[cfg(test)]
mod tests {
    use super::referer_for;

    // manga_parser doesn't currently have a manhuagui.com scraper config, so
    // this special case can't be exercised against a live chapter -- locked
    // in here instead, ported 1:1 from wuxia's lib/util/tools.dart
    // getReferer(): manhuagui.com's CDN wants the image's own URL as
    // Referer, not the chapter page.
    #[test]
    fn manhuagui_uses_image_url_as_referer() {
        let chapter_url = "https://www.manhuagui.com/comic/1234/5678.html";
        let image_url = "https://i.hamreus.com/comic/1234/5678/001.jpg";
        assert_eq!(referer_for(chapter_url, image_url), image_url);
    }

    #[test]
    fn other_sites_use_chapter_page_as_referer() {
        let chapter_url = "https://api.mangadex.org/chapter/dd1fb10a-d0ea-4a09-9cd3-f948a000f1fb";
        let image_url = "https://cmdxd98sb0x3yprd.mangadex.network/data-saver/abc/1.jpg";
        assert_eq!(referer_for(chapter_url, image_url), chapter_url);
    }
}
