//! Full-manga/source image backfill (`Manga.BackfillImages` /
//! `Manga.GetBackfillStatus` in the proto).
//!
//! No new job/queue table: `chapter_image.status` is already a durable,
//! resumable progress ledger -- a backfill is just "keep processing chapters
//! that aren't fully `done` yet". A server restart or a repeated call just
//! continues where it left off. The only in-memory state is a guard set so
//! a duplicate call doesn't spawn a second walker for the same source.

use std::collections::HashSet;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use rand::RngExt;
use sea_orm::{
    ColumnTrait, DatabaseConnection, DbErr, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
    RelationTrait, Statement,
};
use tonic::Status;

use crate::util::chapter_images::{prefetch_chapter, MAX_ATTEMPT_SESSIONS};

lazy_static! {
    static ref BACKFILLING: StdMutex<HashSet<i32>> = StdMutex::new(HashSet::new());
}

fn backfill_chapter_delay_ms() -> u64 {
    std::env::var("BACKFILL_CHAPTER_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500)
}

/// Kick off (or resume) a throttled background walk over every chapter of
/// `manga_source_id`, downloading whatever isn't already `done`. A no-op if
/// a walk for this source is already running.
pub fn start_backfill(db: DatabaseConnection, manga_source_id: i32) {
    {
        let mut running = BACKFILLING.lock().unwrap();
        if !running.insert(manga_source_id) {
            info!(
                "[Backfill] Already running for manga_source {}, ignoring duplicate call",
                manga_source_id
            );
            return;
        }
    }

    tokio::spawn(async move {
        walk(&db, manga_source_id).await;
        BACKFILLING.lock().unwrap().remove(&manga_source_id);
    });
}

async fn walk(db: &DatabaseConnection, manga_source_id: i32) {
    info!("[Backfill] Starting for manga_source {}", manga_source_id);

    let chapters = match entity::chapter::Entity::find()
        .filter(entity::chapter::Column::MangaSourceId.eq(manga_source_id))
        .order_by_asc(entity::chapter::Column::Id)
        .all(db)
        .await
    {
        Ok(chapters) => chapters,
        Err(e) => {
            error!(
                "[Backfill] Failed to list chapters for manga_source {}: {}",
                manga_source_id, e
            );
            return;
        }
    };

    let delay_ms = backfill_chapter_delay_ms();
    let total = chapters.len();

    for (index, chapter) in chapters.into_iter().enumerate() {
        if is_chapter_fully_done(db, chapter.id).await {
            continue;
        }

        info!(
            "[Backfill] ({}/{}) Processing chapter {} [{}]",
            index + 1,
            total,
            chapter.id,
            chapter.url
        );
        prefetch_chapter(db.clone(), chapter).await;

        // Pacing, not concurrency: this is what turns a 3500-chapter
        // backfill into "a couple hours" instead of finishing in well under
        // an hour at full throughput -- rate/pattern is what distinguishes
        // scraper traffic from a human reader, not total volume.
        let jitter: i64 = rand::rng().random_range(-500..=500);
        let sleep_ms = (delay_ms as i64 + jitter).max(0) as u64;
        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
    }

    info!("[Backfill] Finished for manga_source {}", manga_source_id);
}

async fn is_chapter_fully_done(db: &DatabaseConnection, chapter_id: i32) -> bool {
    // A chapter with no rows yet isn't "done" -- it just hasn't been
    // scraped/ensured yet, which `prefetch_chapter` handles.
    let total = entity::chapter_image::Entity::find()
        .filter(entity::chapter_image::Column::ChapterId.eq(chapter_id))
        .count(db)
        .await
        .unwrap_or(0);

    if total == 0 {
        return false;
    }

    let done = entity::chapter_image::Entity::find()
        .filter(entity::chapter_image::Column::ChapterId.eq(chapter_id))
        .filter(entity::chapter_image::Column::Status.eq("done"))
        .count(db)
        .await
        .unwrap_or(0);

    done == total
}

fn auto_backfill_interval_ms() -> u64 {
    std::env::var("AUTO_BACKFILL_INTERVAL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60_000)
}

fn auto_backfill_failure_cooldown_ms() -> i64 {
    std::env::var("AUTO_BACKFILL_FAILURE_COOLDOWN_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(86_400_000)
}

fn auto_backfill_host_backoff_base_ms() -> u64 {
    std::env::var("AUTO_BACKFILL_HOST_BACKOFF_BASE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600_000)
}

fn auto_backfill_batch_size() -> u64 {
    std::env::var("AUTO_BACKFILL_BATCH_SIZE")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(3)
}

/// Slowly, continuously fills in missing chapter images across *every*
/// manga source -- not just the ones a client has explicitly asked to
/// `BackfillImages`, and not just actively-read manga like the eager
/// prefetch in `updater.rs`. Left running for the lifetime of the process,
/// this is what turns the local image store into a full mirror over time,
/// so rumgap can keep serving pages even if a source site goes offline and
/// pulls its images.
///
/// Deliberately separate from `start_backfill`: that's an on-demand,
/// one-source, run-to-completion walk kicked off by a client request. This
/// is an unattended, source-agnostic trickle that wakes up on an interval
/// and only ever claims a small (and odd, so the batch size doesn't read as
/// a suspiciously round bot-shaped number) handful of chapters at a time --
/// same spirit as the pacing in `walk()`, just spread across ticks instead
/// of a per-chapter sleep.
pub async fn watch_auto_backfill(db: DatabaseConnection) {
    let mut interval = tokio::time::interval(Duration::from_millis(auto_backfill_interval_ms()));

    loop {
        interval.tick().await;

        let chapters = match incomplete_chapters(&db, auto_backfill_batch_size()).await {
            Ok(chapters) => chapters,
            Err(e) => {
                error!("[Auto Backfill] Failed to list incomplete chapters: {:#?}", e);
                continue;
            }
        };

        if chapters.is_empty() {
            continue;
        }

        info!("[Auto Backfill] Populating images for {} chapter(s)", chapters.len());
        for chapter in chapters {
            prefetch_chapter(db.clone(), chapter).await;
        }
    }
}

/// Chapters (across every manga source) that don't have a fully `done` set
/// of `chapter_image` rows yet -- either none at all, or some still
/// `pending`/`failed`.
///
/// Ordered least-recently-attempted first (never-attempted chapters before
/// anything else), so chapters that keep failing sink to the back instead of
/// hogging every tick just because they have the lowest ids. Two cooldowns
/// keep failures from being retried too eagerly:
/// - per chapter: a failed image listing is skipped for
///   `AUTO_BACKFILL_FAILURE_COOLDOWN_MS`
/// - per host: every minute with a `chapter_images` failure since the host's
///   last success doubles how long the whole host is skipped, starting at
///   `AUTO_BACKFILL_HOST_BACKOFF_BASE_MS` and uncapped (the exponent is
///   clamped at 1000 only so `float8` can't overflow), so a dead site gets
///   probed less and less instead of eating the whole trickle
///
/// A chapter stuck on a durably-failed page just rides along in the batch
/// for free (`ensure_page_downloaded`'s own cooldown/attempt cap makes that
/// a cheap no-op) without blocking the others.
async fn incomplete_chapters(db: &DatabaseConnection, limit: u64) -> Result<Vec<entity::chapter::Model>, DbErr> {
    let now = chrono::Utc::now().naive_utc();
    let listing_failure_cutoff = now - chrono::Duration::milliseconds(auto_backfill_failure_cooldown_ms());

    let statement = Statement::from_sql_and_values(
        db.get_database_backend(),
        r#"
        WITH last_host_success AS (
            SELECT hostname, max(created_at) AS succeeded_at
            FROM scrape_log
            WHERE operation = 'chapter_images' AND success
            GROUP BY hostname
        ),
        host_failures AS (
            SELECT l.hostname, count(DISTINCT date_trunc('minute', l.created_at)) AS failures,
                max(l.created_at) AS failed_at
            FROM scrape_log l
            LEFT JOIN last_host_success s ON s.hostname = l.hostname
            WHERE l.operation = 'chapter_images' AND NOT l.success
              AND (s.succeeded_at IS NULL OR l.created_at > s.succeeded_at)
            GROUP BY l.hostname
        ),
        backed_off_hosts AS (
            SELECT hostname
            FROM host_failures
            WHERE extract(epoch FROM $2 - failed_at) < $1::float8 * power(2::float8, LEAST(failures, 1000) - 1)
        ),
        last_attempt AS (
            SELECT url, max(created_at) AS attempted_at
            FROM scrape_log
            WHERE operation = 'chapter_images'
            GROUP BY url
        )
        SELECT chapter.*
        FROM chapter
        LEFT JOIN chapter_image ON chapter_image.chapter_id = chapter.id
        LEFT JOIN last_attempt ON last_attempt.url = chapter.url
        WHERE NOT EXISTS (
            SELECT 1 FROM backed_off_hosts
            WHERE backed_off_hosts.hostname = lower(substring(chapter.url FROM '^[^:]+://(?:[^@/]*@)?([^/:?#]+)'))
        )
        AND NOT EXISTS (
            SELECT 1 FROM scrape_log
            WHERE scrape_log.url = chapter.url AND scrape_log.operation = 'chapter_images'
              AND NOT scrape_log.success AND scrape_log.created_at > $3
        )
        GROUP BY chapter.id, last_attempt.attempted_at
        HAVING COUNT(chapter_image.chapter_id) = 0
            OR COUNT(chapter_image.chapter_id) FILTER (WHERE chapter_image.status <> 'done'
                AND NOT (chapter_image.status = 'failed' AND chapter_image.attempts >= $4)) > 0
        ORDER BY last_attempt.attempted_at ASC NULLS FIRST, chapter.id ASC
        LIMIT $5
        "#,
        [
            (auto_backfill_host_backoff_base_ms() as f64 / 1000.0).into(),
            now.into(),
            listing_failure_cutoff.into(),
            MAX_ATTEMPT_SESSIONS.into(),
            (limit as i64).into(),
        ],
    );

    entity::chapter::Entity::find().from_raw_sql(statement).all(db).await
}

/// `images_downloaded`/`images_total` for `GetBackfillStatus` -- a cheap
/// count across every chapter belonging to the source, polled by the client
/// on its own schedule rather than a long-lived streaming RPC.
pub async fn backfill_status(db: &DatabaseConnection, manga_source_id: i32) -> Result<(i32, i32), Status> {
    let total = entity::chapter_image::Entity::find()
        .join(
            migration::JoinType::InnerJoin,
            entity::chapter_image::Relation::Chapter.def(),
        )
        .filter(entity::chapter::Column::MangaSourceId.eq(manga_source_id))
        .count(db)
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

    let downloaded = entity::chapter_image::Entity::find()
        .join(
            migration::JoinType::InnerJoin,
            entity::chapter_image::Relation::Chapter.def(),
        )
        .filter(entity::chapter::Column::MangaSourceId.eq(manga_source_id))
        .filter(entity::chapter_image::Column::Status.eq("done"))
        .count(db)
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

    Ok((downloaded as i32, total as i32))
}
