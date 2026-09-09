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

use migration::{Expr, JoinType};
use rand::Rng;
use sea_orm::{
    ColumnTrait, DatabaseConnection, DbErr, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
    RelationTrait,
};
use tonic::Status;

use crate::util::chapter_images::prefetch_chapter;

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
        let jitter: i64 = rand::thread_rng().gen_range(-500..=500);
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
/// `pending`/`failed`. Ordered by id so the trickle makes steady forward
/// progress through the whole table instead of re-rolling the same random
/// sample every tick; a chapter stuck on a durably-failed page just rides
/// along in the batch for free (`ensure_page_downloaded`'s own cooldown/
/// attempt cap makes that a cheap no-op) without blocking the others.
async fn incomplete_chapters(db: &DatabaseConnection, limit: u64) -> Result<Vec<entity::chapter::Model>, DbErr> {
    entity::chapter::Entity::find()
        .join(JoinType::LeftJoin, entity::chapter::Relation::ChapterImage.def())
        .group_by(entity::chapter::Column::Id)
        .having(Expr::cust(
            "COUNT(chapter_image.chapter_id) = 0 \
             OR COUNT(chapter_image.chapter_id) != COUNT(chapter_image.chapter_id) FILTER (WHERE chapter_image.status = 'done')",
        ))
        .order_by_asc(entity::chapter::Column::Id)
        .limit(limit)
        .all(db)
        .await
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
