//! Persists every `manga_parser` scrape attempt (success or failure) to `scrape_log`, feeding
//! the admin-only `Scraper.Status` RPC (`service::v1::scraper`). The point is to notice a
//! scraper silently breaking -- a site changed its markup, started Cloudflare-gating, etc --
//! without having to grep log files for a hostname you suspect might be failing.

use std::future::Future;
use std::time::Instant;

use manga_parser::error::ScrapeError;
use manga_parser::Url;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use tonic::Status;

use crate::util::scrape_error_proto::StatusWrapper;

/// A successful attempt is only useful to confirm "still working" -- kept just long enough to
/// compute a rolling attempts/failures ratio. Failures are the whole point of this table
/// (spotting a scraper that's been consistently broken for weeks), so they're kept far longer.
const SUCCESS_RETENTION_DAYS: i64 = 3;
const FAILURE_RETENTION_DAYS: i64 = 60;

/// Run a `manga_parser` scrape against `url`, recording its outcome (success/failure, timing,
/// error detail) to `scrape_log`, then convert any error to a `Status` the same way every
/// scrape call site already did before this wrapper existed (see `StatusWrapper`).
///
/// `manga_id`/`manga_source_id` are best-effort context for the log row (not always known yet,
/// e.g. a brand new source hasn't been inserted at scrape time) -- pass `None` when unknown.
pub async fn record<T, F>(
    db: &DatabaseConnection,
    operation: &'static str,
    url: &Url,
    manga_id: Option<i32>,
    manga_source_id: Option<i32>,
    fut: F,
) -> Result<T, Status>
where
    F: Future<Output = Result<T, ScrapeError>>,
{
    let hostname = url.host_str().unwrap_or("unknown").to_string();
    let start = Instant::now();
    let result = fut.await;
    let duration_ms = start.elapsed().as_millis().min(i32::MAX as u128) as i32;

    let (success, error_type, error_message) = match &result {
        Ok(_) => (true, None, None),
        Err(e) => (false, Some(e.as_ref().to_string()), Some(e.to_string())),
    };

    let log = entity::scrape_log::ActiveModel {
        hostname: Set(hostname.clone()),
        operation: Set(operation.to_string()),
        url: Set(url.to_string()),
        manga_id: Set(manga_id),
        manga_source_id: Set(manga_source_id),
        success: Set(success),
        error_type: Set(error_type),
        error_message: Set(error_message),
        duration_ms: Set(duration_ms),
        ..Default::default()
    };

    if let Err(e) = log.insert(db).await {
        warn!("Failed to write scrape_log row for {}: {}", hostname, e);
    }

    result.map_err(|e| StatusWrapper::from(e).into())
}

/// Record a suspicious chapter-count drop (see `service::v1::manga::sync_chapters_for_source`)
/// as a `scrape_log` failure so it surfaces on the admin `Scraper.Status` page for a human to go
/// check the url themselves, instead of the old behaviour of silently trusting the scrape and
/// deleting every chapter for the source.
///
/// This isn't a `ScrapeError` -- the scrape itself succeeded -- so unlike `record`, there's no
/// `error_type` to set.
pub async fn flag_suspicious_chapter_drop(
    db: &DatabaseConnection,
    manga_id: i32,
    manga_source_id: i32,
    url: &str,
    scraped_count: u64,
    previous_count: u64,
) {
    let hostname = Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string());

    let log = entity::scrape_log::ActiveModel {
        hostname: Set(hostname),
        operation: Set("manga".to_string()),
        url: Set(url.to_string()),
        manga_id: Set(Some(manga_id)),
        manga_source_id: Set(Some(manga_source_id)),
        success: Set(false),
        error_type: Set(None),
        error_message: Set(Some(format!(
            "Scraped {scraped_count} chapter(s), expected ~{previous_count} - possible false chapter \
             removal, skipped auto-reset"
        ))),
        duration_ms: Set(0),
        ..Default::default()
    };

    if let Err(e) = log.insert(db).await {
        warn!("Failed to write scrape_log row for suspicious chapter drop on {}: {}", url, e);
    }
}

/// Delete old `scrape_log` rows. Called periodically from the auto-update loop rather than on
/// every scrape -- it's pure maintenance and doesn't need to sit on the hot scrape path.
pub async fn prune(db: &DatabaseConnection) {
    let now = chrono::Utc::now().naive_utc();

    let success_cutoff = now - chrono::Duration::days(SUCCESS_RETENTION_DAYS);
    let deleted_success = entity::scrape_log::Entity::delete_many()
        .filter(entity::scrape_log::Column::Success.eq(true))
        .filter(entity::scrape_log::Column::CreatedAt.lt(success_cutoff))
        .exec(db)
        .await;
    match deleted_success {
        Ok(res) if res.rows_affected > 0 => {
            info!("Pruned {} successful scrape_log row(s)", res.rows_affected);
        }
        Err(e) => warn!("Failed to prune successful scrape_log rows: {}", e),
        _ => {}
    }

    let failure_cutoff = now - chrono::Duration::days(FAILURE_RETENTION_DAYS);
    let deleted_failure = entity::scrape_log::Entity::delete_many()
        .filter(entity::scrape_log::Column::Success.eq(false))
        .filter(entity::scrape_log::Column::CreatedAt.lt(failure_cutoff))
        .exec(db)
        .await;
    match deleted_failure {
        Ok(res) if res.rows_affected > 0 => {
            info!("Pruned {} failed scrape_log row(s)", res.rows_affected);
        }
        Err(e) => warn!("Failed to prune failed scrape_log rows: {}", e),
        _ => {}
    }
}
