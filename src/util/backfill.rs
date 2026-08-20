//! Full-manga/source image backfill (`Chapter.BackfillImages` /
//! `Chapter.GetBackfillStatus` in the proto -- see the deviation note in
//! `proto/rumgap/v1/v1.proto` for why these live there instead of on a
//! `MangaSource` service).
//!
//! No new job/queue table: `chapter_image.status` is already a durable,
//! resumable progress ledger -- a backfill is just "keep processing chapters
//! that aren't fully `done` yet". A server restart or a repeated call just
//! continues where it left off. The only in-memory state is a guard set so
//! a duplicate call doesn't spawn a second walker for the same source.

use std::collections::HashSet;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use rand::Rng;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, RelationTrait,
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
///
/// NOTE: until Phase 1's `manga_source` table lands in this codebase,
/// `manga_source_id` is resolved against `manga.id` (today's 1:1
/// manga<->source shape) -- chapters are filtered by `chapter.manga_id`.
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
        .filter(entity::chapter::Column::MangaId.eq(manga_source_id))
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

/// `images_downloaded`/`images_total` for `GetBackfillStatus` -- a cheap
/// count across every chapter belonging to the source, polled by the client
/// on its own schedule rather than a long-lived streaming RPC.
pub async fn backfill_status(db: &DatabaseConnection, manga_source_id: i32) -> Result<(i32, i32), Status> {
    let total = entity::chapter_image::Entity::find()
        .join(migration::JoinType::InnerJoin, entity::chapter_image::Relation::Chapter.def())
        .filter(entity::chapter::Column::MangaId.eq(manga_source_id))
        .count(db)
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

    let downloaded = entity::chapter_image::Entity::find()
        .join(migration::JoinType::InnerJoin, entity::chapter_image::Relation::Chapter.def())
        .filter(entity::chapter::Column::MangaId.eq(manga_source_id))
        .filter(entity::chapter_image::Column::Status.eq("done"))
        .count(db)
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

    Ok((downloaded as i32, total as i32))
}
