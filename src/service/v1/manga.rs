use std::collections::HashMap;
use std::pin::Pin;
use std::time::Duration;

use chrono::{NaiveDateTime, Utc};
use futures::Stream;
use manga_parser::scraper::MangaScraper;
use manga_parser::Url;
use migration::{Expr, ExprTrait, JoinType, OnConflict};
use sea_orm::ActiveValue::{NotSet, Set};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, QueryTrait, RelationTrait, Select, TransactionTrait,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status};

use crate::interceptor::auth::{UserHasPermissions, UserPermissions};
use crate::proto::manga_server::{Manga, MangaServer};
use crate::proto::{
    AddSourceRequest, BackfillImagesRequest, BackfillStatusReply, Empty, GetBackfillStatusRequest, GetMangaRequest,
    MangaReply, MangaRequest, MangaSourceReply, MangasReply, MangasRequest, MergeMangaRequest, MoveSourceRequest,
    PaginateReply, PaginateSearchQuery, RemoveSourceRequest, SetPrimarySourceRequest, SimilarMangaRequest,
    UpdateMangaRequest,
};
use crate::service::v1::reading::sync_reading_progress;
use crate::util::auth::Authorize;
use crate::util::backfill;
use crate::util::db::DatabaseRequest;
use crate::util::search::manga::lucene_filter;
use crate::{data, util, MANGA_PARSER};

type ResponseStream = Pin<Box<dyn Stream<Item = Result<MangaReply, Status>> + Send>>;

pub const NEXT_UPDATE_QUERY: &str =
    "(MAX(chapter.posted) + (MAX(chapter.posted) - MIN(chapter.posted)) / NULLIF(COUNT(*) - 1, 0))";

fn internal<E: ToString>(e: E) -> Status {
    Status::internal(e.to_string())
}

/// Derive a hostname from a URL the same way the `create_manga_source` migration's backfill
/// does (`substring(url from '://([^/]+)')`), so freshly-added sources look the same as
/// backfilled ones.
fn hostname_of(url: &Url) -> String {
    url.host_str().unwrap_or("unknown").to_string()
}

/// Restricts a manga -> manga_source join to just the primary source, since count_chapters/
/// last/next are computed from the primary source only.
fn join_primary_source_and_chapters(query: Select<entity::manga::Entity>) -> Select<entity::manga::Entity> {
    query
        .join(
            JoinType::LeftJoin,
            entity::manga::Relation::MangaSource.def().on_condition(|_left, right| {
                Expr::col((right, entity::manga_source::Column::IsPrimary))
                    .eq(true)
                    .into()
            }),
        )
        .join(JoinType::LeftJoin, entity::manga_source::Relation::Chapter.def())
}

/// Batch-fetch every manga_source for the given manga ids, grouped by manga_id. Used to
/// populate `MangaReply.sources` after the flat aggregate query for the rest of the reply.
pub async fn load_manga_sources(
    db: &DatabaseConnection,
    manga_ids: &[i32],
) -> Result<HashMap<i32, Vec<MangaSourceReply>>, Status> {
    if manga_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let sources = entity::manga_source::Entity::find()
        .filter(entity::manga_source::Column::MangaId.is_in(manga_ids.to_vec()))
        .order_by_desc(entity::manga_source::Column::IsPrimary)
        .order_by_asc(entity::manga_source::Column::Id)
        .all(db)
        .await
        .map_err(internal)?;

    let mut map: HashMap<i32, Vec<MangaSourceReply>> = HashMap::new();
    for source in sources {
        map.entry(source.manga_id).or_default().push(source.into());
    }
    Ok(map)
}

async fn load_manga_sources_for(db: &DatabaseConnection, manga_id: i32) -> Result<Vec<MangaSourceReply>, Status> {
    Ok(load_manga_sources(db, &[manga_id])
        .await?
        .remove(&manga_id)
        .unwrap_or_default())
}

async fn attach_sources(db: &DatabaseConnection, items: Vec<data::manga::Full>) -> Result<Vec<MangaReply>, Status> {
    let ids: Vec<i32> = items.iter().map(|item| item.id).collect();
    let mut sources_by_manga = load_manga_sources(db, &ids).await?;

    Ok(items
        .into_iter()
        .map(|item| {
            let sources = sources_by_manga.remove(&item.id).unwrap_or_default();
            item.into_manga_reply(sources)
        })
        .collect())
}

/// Get a "full" manga by it's ID
#[rustfmt::skip]
pub async fn get_manga_by_id(db: &DatabaseConnection, logged_in: Option<&entity::user::Model>, manga_id: i32) -> Result<MangaReply, Status> {
    use entity::chapter::Column as ChapterColumn;

    let manga = join_primary_source_and_chapters(entity::manga::Entity::find_by_id(manga_id))
        .column_as(ChapterColumn::Id.count(), "count_chapters")
        .column_as(ChapterColumn::Posted.max(), "last")
        .column_as(Expr::cust(NEXT_UPDATE_QUERY), "next")
        .group_by(entity::manga::Column::Id)
        .column_as(Expr::cust("null"), "progress")
        .column_as(Expr::cust("null"), "progress_ordinal")
        .apply_if(logged_in, |query, logged_in| {
            let user_id = logged_in.id;
            query
                .join(
                JoinType::LeftJoin,
                entity::reading::Relation::Manga.def().rev().on_condition(
                        move |_left, right| {
                            Expr::col((right, entity::reading::Column::UserId))
                                .eq(user_id)
                                .into()
                        },
                    ),
                )
                .join(JoinType::LeftJoin, entity::reading::Relation::CanonicalChapter.def())
                .column_as(entity::reading::Column::Progress, "progress")
                .column_as(entity::canonical_chapter::Column::Ordinal, "progress_ordinal")
                .group_by(entity::reading::Column::UserId)
                .group_by(entity::reading::Column::MangaId)
                .group_by(entity::canonical_chapter::Column::Ordinal)
        })
        .into_model::<data::manga::Full>()
        .one(db)
        .await
        .map_err(internal)?
        .ok_or(Status::not_found("Manga not found"))?;

    let sources = load_manga_sources_for(db, manga_id).await?;

    Ok(manga.into_manga_reply(sources))
}

/// Run the chapter <-> canonical_chapter matching heuristic (Phase 1b) on the given
/// (freshly-inserted) chapters. Deliberately simple, no fuzzy scoring: find an existing
/// canonical_chapter for this manga whose `ordinal` equals the chapter's `number` and link
/// to it; if none exists, create one and link to it. Every chapter always ends up attached
/// to *some* canonical_chapter this way, including multiple chapters from the same source
/// sharing an ordinal (e.g. two scanlation groups both releasing "chapter 5") - they all
/// link to the same canonical_chapter, which is exactly what `LinkChapter`/`UnlinkChapter`
/// exist to override manually if a particular pairing turns out wrong.
async fn match_canonical_chapters<C: ConnectionTrait>(
    db: &C,
    manga_id: i32,
    chapters: &[entity::chapter::Model],
) -> Result<(), Status> {
    for chapter in chapters {
        let ordinal = sea_orm::prelude::Decimal::from_f32_retain(chapter.number)
            .unwrap_or_default()
            .round_dp(3);

        let canonical_id = find_or_create_canonical_chapter(db, manga_id, ordinal).await?;

        entity::chapter::ActiveModel {
            id: Set(chapter.id),
            canonical_chapter_id: Set(Some(canonical_id)),
            ..Default::default()
        }
        .update(db)
        .await
        .map_err(internal)?;
    }

    Ok(())
}

/// Finds the manga's canonical_chapter row at this ordinal, creating one if none exists
/// yet. Shared by match_canonical_chapters (per freshly-inserted chapter) and the
/// merge/move-source path (remapping a chapter/reading row already linked under a
/// *different* manga onto this one's equivalent ordinal).
async fn find_or_create_canonical_chapter<C: ConnectionTrait>(
    db: &C,
    manga_id: i32,
    ordinal: sea_orm::prelude::Decimal,
) -> Result<i32, Status> {
    let existing = entity::canonical_chapter::Entity::find()
        .filter(entity::canonical_chapter::Column::MangaId.eq(manga_id))
        .filter(entity::canonical_chapter::Column::Ordinal.eq(ordinal))
        .one(db)
        .await
        .map_err(internal)?;

    match existing {
        Some(existing) => Ok(existing.id),
        None => Ok(entity::canonical_chapter::ActiveModel {
            manga_id: Set(manga_id),
            ordinal: Set(ordinal),
            ..Default::default()
        }
        .insert(db)
        .await
        .map_err(internal)?
        .id),
    }
}

/// If `manga_id` still has another source once `excluding_source_id` is removed/moved
/// away, promotes it to primary - so canonical manga metadata is never left without a
/// primary source backing it. No-op if that was the manga's only source.
async fn promote_replacement_primary<C: ConnectionTrait>(
    db: &C,
    manga_id: i32,
    excluding_source_id: i32,
) -> Result<(), Status> {
    let replacement = entity::manga_source::Entity::find()
        .filter(entity::manga_source::Column::MangaId.eq(manga_id))
        .filter(entity::manga_source::Column::Id.ne(excluding_source_id))
        .order_by_asc(entity::manga_source::Column::Id)
        .one(db)
        .await
        .map_err(internal)?;

    if let Some(replacement) = replacement {
        entity::manga_source::ActiveModel {
            id: Set(replacement.id),
            is_primary: Set(true),
            ..Default::default()
        }
        .update(db)
        .await
        .map_err(internal)?;

        info!(
            "Auto-promoted manga_source {} to primary (excluded source {} from manga {})",
            replacement.id, excluding_source_id, manga_id
        );
    }

    Ok(())
}

/// Reconciles `reading` rows when manga_id `from` is about to disappear into `to`. The
/// composite PK (user_id, manga_id) means a user with progress on both sides collides -
/// per user, keeps whichever side has read further (by `progress`), remapping its
/// `last_canonical_chapter_id` onto `to`'s equivalent-ordinal canonical row (via
/// find_or_create_canonical_chapter + sync_reading_progress, so progress and
/// last_canonical_chapter_id never drift apart) before dropping the `from` row. A user
/// with a row on only one side is simply carried over the same way.
async fn merge_reading_progress<C: ConnectionTrait>(db: &C, from_manga_id: i32, to_manga_id: i32) -> Result<(), Status> {
    let from_rows = entity::reading::Entity::find()
        .filter(entity::reading::Column::MangaId.eq(from_manga_id))
        .all(db)
        .await
        .map_err(internal)?;

    for from_row in from_rows {
        let to_row = entity::reading::Entity::find_by_id((from_row.user_id, to_manga_id))
            .one(db)
            .await
            .map_err(internal)?;

        let from_wins = match &to_row {
            Some(to_row) => from_row.progress > to_row.progress,
            None => true,
        };

        if from_wins {
            let remapped_canonical_id = match from_row.last_canonical_chapter_id {
                Some(old_canonical_id) => {
                    let ordinal = entity::canonical_chapter::Entity::find_by_id(old_canonical_id)
                        .one(db)
                        .await
                        .map_err(internal)?
                        .map(|c| c.ordinal);

                    match ordinal {
                        Some(ordinal) => Some(find_or_create_canonical_chapter(db, to_manga_id, ordinal).await?),
                        None => None,
                    }
                }
                None => None,
            };

            if to_row.is_some() {
                entity::reading::ActiveModel {
                    user_id: Set(from_row.user_id),
                    manga_id: Set(to_manga_id),
                    progress: Set(from_row.progress),
                    last_canonical_chapter_id: Set(remapped_canonical_id),
                    ..Default::default()
                }
                .update(db)
                .await
                .map_err(internal)?;
            } else {
                entity::reading::ActiveModel {
                    user_id: Set(from_row.user_id),
                    manga_id: Set(to_manga_id),
                    progress: Set(from_row.progress),
                    last_canonical_chapter_id: Set(remapped_canonical_id),
                    ..Default::default()
                }
                .insert(db)
                .await
                .map_err(internal)?;
            }

            if let Some(remapped_canonical_id) = remapped_canonical_id {
                // Recompute `progress` as `to`'s own canonical rank rather than trusting the
                // raw count carried over from `from` (ranked against a different manga's
                // canonical rows) - the one shared place progress/last_canonical_chapter_id
                // are ever written together.
                sync_reading_progress(db, from_row.user_id, to_manga_id, remapped_canonical_id).await?;
            }
        }

        entity::reading::Entity::delete_by_id((from_row.user_id, from_manga_id))
            .exec(db)
            .await
            .map_err(internal)?;
    }

    Ok(())
}

/// Moves a single manga_source (and its chapters) onto a different manga: re-parents
/// manga_source.manga_id, demotes it to non-primary (the target already has its own
/// primary source), and re-links its chapters' canonical_chapter_id onto the target
/// manga's canonical rows via match_canonical_chapters - exactly as if it had just been
/// freshly added there. If this emptied out the source manga's last remaining source,
/// that manga is now a dead husk: its reading progress is folded into the target (see
/// merge_reading_progress) and it's deleted, mirroring RemoveSource's existing
/// "delete once sourceless" rule.
async fn move_source_to_manga<C: ConnectionTrait>(db: &C, manga_source_id: i32, target_manga_id: i32) -> Result<(), Status> {
    let source = entity::manga_source::Entity::find_by_id(manga_source_id)
        .one(db)
        .await
        .map_err(internal)?
        .ok_or(Status::not_found("Manga source not found"))?;

    let old_manga_id = source.manga_id;
    if old_manga_id == target_manga_id {
        return Err(Status::invalid_argument("Source already belongs to the target manga"));
    }

    entity::manga::Entity::find_by_id(target_manga_id)
        .one(db)
        .await
        .map_err(internal)?
        .ok_or(Status::not_found("Target manga not found"))?;

    if source.is_primary {
        promote_replacement_primary(db, old_manga_id, source.id).await?;
    }

    let chapters = entity::chapter::Entity::find()
        .filter(entity::chapter::Column::MangaSourceId.eq(manga_source_id))
        .all(db)
        .await
        .map_err(internal)?;

    entity::manga_source::ActiveModel {
        id: Set(source.id),
        manga_id: Set(target_manga_id),
        is_primary: Set(false),
        ..Default::default()
    }
    .update(db)
    .await
    .map_err(internal)?;

    match_canonical_chapters(db, target_manga_id, &chapters).await?;

    let remaining_sources = entity::manga_source::Entity::find()
        .filter(entity::manga_source::Column::MangaId.eq(old_manga_id))
        .count(db)
        .await
        .map_err(internal)?;

    if remaining_sources == 0 {
        merge_reading_progress(db, old_manga_id, target_manga_id).await?;

        entity::manga::Entity::delete_by_id(old_manga_id)
            .exec(db)
            .await
            .map_err(internal)?;

        info!(
            "Deleted manga {} - its last source just moved to manga {}",
            old_manga_id, target_manga_id
        );
    }

    Ok(())
}

/// Merges every source of `source_manga_id` into `target_manga_id`, implemented as
/// move_source_to_manga per source - once the loop empties source_manga_id out, that last
/// call's own cleanup (reading merge + delete) finishes the job, so there's no separate
/// "delete the manga" step here.
async fn merge_manga_sources<C: ConnectionTrait>(db: &C, source_manga_id: i32, target_manga_id: i32) -> Result<(), Status> {
    entity::manga::Entity::find_by_id(target_manga_id)
        .one(db)
        .await
        .map_err(internal)?
        .ok_or(Status::not_found("Target manga not found"))?;

    let sources = entity::manga_source::Entity::find()
        .filter(entity::manga_source::Column::MangaId.eq(source_manga_id))
        .all(db)
        .await
        .map_err(internal)?;

    if sources.is_empty() {
        return Err(Status::not_found("Source manga not found, or has no sources"));
    }

    for source in sources {
        move_source_to_manga(db, source.id, target_manga_id).await?;
    }

    Ok(())
}

/// A chapter-count drop this large (as a fraction of what we had) is treated as suspicious
/// rather than a legitimate renumbering - e.g. a source flipping to a single "removed" notice
/// page would otherwise read as "chapters legitimately shrank" and wipe real data.
const SUSPICIOUS_CHAPTER_DROP_RATIO: f64 = 0.5;

/// Insert/reconcile chapters scraped for a single manga_source, then run the canonical
/// matching heuristic on exactly the newly-inserted rows (never touching chapters that
/// already have a link, whether auto-matched before or manually unlinked).
async fn sync_chapters_for_source(
    db: &DatabaseConnection,
    manga_id: i32,
    manga_source_id: i32,
    source_url: &str,
    force: bool,
    scraped_chapters: &[manga_parser::model::Chapter],
) -> Result<(), Status> {
    if scraped_chapters.is_empty() {
        error!("No chapters found for manga_source {}", manga_source_id);
        return Ok(());
    }

    let count_chapters = entity::chapter::Entity::find()
        .filter(entity::chapter::Column::MangaSourceId.eq(manga_source_id))
        .count(db)
        .await
        .map_err(internal)?;

    if (scraped_chapters.len() as u64) < count_chapters {
        let dropped = count_chapters - scraped_chapters.len() as u64;
        let drop_ratio = dropped as f64 / count_chapters as f64;

        if drop_ratio > SUSPICIOUS_CHAPTER_DROP_RATIO && !force {
            // Too big a drop to trust - don't touch existing chapters, just flag it so a
            // human can go check the url themselves.
            warn!(
                "Suspicious chapter drop for manga_source {} [{}]: scraped {} chapter(s), had {} - skipping reset",
                manga_source_id,
                source_url,
                scraped_chapters.len(),
                count_chapters
            );
            util::scrape_log::flag_suspicious_chapter_drop(
                db,
                manga_id,
                manga_source_id,
                source_url,
                scraped_chapters.len() as u64,
                count_chapters,
            )
            .await;
            return Ok(());
        }

        if drop_ratio > SUSPICIOUS_CHAPTER_DROP_RATIO {
            info!(
                "Forcing chapter reset for manga_source {} [{}] despite suspicious drop ({} -> {})",
                manga_source_id,
                source_url,
                count_chapters,
                scraped_chapters.len()
            );
        }

        // A modest drop in chapter count (or a forced refresh) - reset our chapters to match the source.
        let res = entity::chapter::Entity::delete_many()
            .filter(entity::chapter::Column::MangaSourceId.eq(manga_source_id))
            .exec(db)
            .await
            .map_err(internal)?;
        info!("Cleared {} chapter(s)", res.rows_affected);
    }

    let mut chapters = vec![];
    for chapter in scraped_chapters.iter().rev() {
        chapters.push(entity::chapter::ActiveModel {
            manga_source_id: Set(manga_source_id),
            number: Set(chapter.number),
            url: Set(chapter.url.to_string()),
            title: Set(chapter.title.clone()),
            posted: Set(chapter.date.map(|date| date.into())),
            ..Default::default()
        });
    }
    info!("Inserting {} chapter(s)", chapters.len());

    // ON CONFLICT DO NOTHING + RETURNING gives back exactly the newly-inserted rows (in
    // Postgres), which is exactly what the canonical matching heuristic needs to run
    // against - never re-touching chapters that were already linked (or manually unlinked).
    let inserted = entity::chapter::Entity::insert_many(chapters)
        .on_conflict(OnConflict::column(entity::chapter::Column::Url).do_nothing().to_owned())
        .exec_with_returning(db)
        .await
        .map_err(internal)?;

    info!("Inserted {} unique chapter(s)", inserted.len());

    match_canonical_chapters(db, manga_id, &inserted).await
}

/// Create a brand-new canonical manga from a freshly-scraped URL, with this as its (first,
/// and therefore primary) source.
pub async fn create_manga(db: &DatabaseConnection, url: Url) -> Result<i32, Status> {
    info!("Creating manga [{}]", url);

    let manga: manga_parser::model::Manga =
        crate::util::scrape_log::record(db, "manga", &url, None, None, MANGA_PARSER.manga(&url)).await?;

    let saved_manga = entity::manga::ActiveModel {
        title: Set(manga.title),
        description: Set(manga.description),
        is_ongoing: Set(manga.is_ongoing),
        cover_source_url: Set(manga.cover_url.map(|url| url.to_string())),
        authors: Set(manga.authors),
        alt_titles: Set(manga.alternative_titles),
        genres: Set(manga.genres),
        status: manga.status.map_or(NotSet, Set),
        ..Default::default()
    }
    .insert(db)
    .await
    .map_err(internal)?;

    let saved_source = entity::manga_source::ActiveModel {
        manga_id: Set(saved_manga.id),
        url: Set(manga.url.to_string()),
        hostname: Set(hostname_of(&manga.url)),
        is_primary: Set(true),
        ..Default::default()
    }
    .insert(db)
    .await
    .map_err(internal)?;

    sync_chapters_for_source(db, saved_manga.id, saved_source.id, &saved_source.url, false, &manga.chapters).await?;

    Ok(saved_manga.id)
}

/// Add a new (by default, secondary) source to an existing manga: scrapes via the existing
/// call path, creates the manga_source row, inserts chapters against it, runs the matching
/// heuristic. Only the *primary* source's scrape updates canonical manga fields, so a brand
/// new non-primary source never touches them.
pub async fn add_manga_source(db: &DatabaseConnection, manga_id: i32, url: Url) -> Result<i32, Status> {
    entity::manga::Entity::find_by_id(manga_id)
        .one(db)
        .await
        .map_err(internal)?
        .ok_or(Status::not_found("Manga not found"))?;

    let existing = entity::manga_source::Entity::find()
        .filter(entity::manga_source::Column::Url.eq(url.to_string()))
        .one(db)
        .await
        .map_err(internal)?;
    if existing.is_some() {
        return Err(Status::already_exists("A source with this url already exists"));
    }

    info!("Adding source [{}] to manga {}", url, manga_id);

    let manga: manga_parser::model::Manga =
        crate::util::scrape_log::record(db, "manga", &url, Some(manga_id), None, MANGA_PARSER.manga(&url)).await?;

    let saved_source = entity::manga_source::ActiveModel {
        manga_id: Set(manga_id),
        url: Set(manga.url.to_string()),
        hostname: Set(hostname_of(&manga.url)),
        is_primary: Set(false),
        ..Default::default()
    }
    .insert(db)
    .await
    .map_err(internal)?;

    sync_chapters_for_source(db, manga_id, saved_source.id, &saved_source.url, false, &manga.chapters).await?;

    Ok(saved_source.id)
}

/// Refresh an existing manga_source in place (re-scrape its url). If it's the primary
/// source, canonical manga.title/description/cover/etc are updated too - a secondary
/// source's refresh must never stomp canonical metadata.
///
/// `force` bypasses the suspicious-chapter-drop safety net in `sync_chapters_for_source` --
/// only the explicit user-triggered `Manga.Update` RPC should ever pass `true`, once someone
/// has actually checked the source and confirmed the drop is real.
pub async fn refresh_manga_source(db: &DatabaseConnection, manga_source_id: i32, force: bool) -> Result<i32, Status> {
    let source = entity::manga_source::Entity::find_by_id(manga_source_id)
        .one(db)
        .await
        .map_err(internal)?
        .ok_or(Status::not_found("Manga source not found"))?;

    let url = Url::parse(&source.url).map_err(|e| Status::invalid_argument(e.to_string()))?;
    info!("Refreshing manga source {} [{}]", manga_source_id, url);

    let manga: manga_parser::model::Manga = crate::util::scrape_log::record(
        db,
        "manga",
        &url,
        Some(source.manga_id),
        Some(manga_source_id),
        MANGA_PARSER.manga(&url),
    )
    .await?;

    if source.is_primary {
        // Only overwrite the cover when this scrape actually found one - cover_url is a
        // genuinely optional field in manga_parser's model (unlike title/description, which
        // are required on the builder and fail the whole scrape if unextractable), so a
        // successful-but-cover-less scrape must not silently wipe a previously-good cover
        // with NULL.
        let new_cover_source_url = manga.cover_url.clone().map(|url| url.to_string());

        let mut active = entity::manga::ActiveModel {
            id: Set(source.manga_id),
            title: Set(manga.title.clone()),
            description: Set(manga.description.clone()),
            is_ongoing: Set(manga.is_ongoing),
            authors: Set(manga.authors.clone()),
            alt_titles: Set(manga.alternative_titles.clone()),
            genres: Set(manga.genres.clone()),
            status: manga.status.clone().map_or(NotSet, Set),
            ..Default::default()
        };

        if let Some(new_cover_source_url) = new_cover_source_url {
            let current = entity::manga::Entity::find_by_id(source.manga_id)
                .one(db)
                .await
                .map_err(internal)?
                .ok_or(Status::not_found("Manga not found"))?;

            // A source flip (e.g. SetPrimarySource) or the primary source's own cover
            // changing means whatever's cached under the OLD cover_source_url no longer
            // applies - reset the download state so the new URL actually gets fetched,
            // instead of `cover_status == "done"` short-circuiting `ensure_cover_downloaded`
            // forever under the now-wrong assumption that "done" still refers to this URL.
            if current.cover_source_url.as_deref() != Some(new_cover_source_url.as_str()) {
                if let Some(old_key) = &current.cover_storage_key {
                    if let Err(e) = crate::IMAGE_STORE.delete(old_key).await {
                        warn!("Failed to delete stale cover {}: {}", old_key, e);
                    }
                }
                active.cover_status = Set("pending".to_string());
                active.cover_storage_key = Set(None);
                active.cover_content_type = Set(None);
                active.cover_checksum = Set(None);
                active.cover_attempts = Set(0);
            }
            active.cover_source_url = Set(Some(new_cover_source_url));
        }

        active.update(db).await.map_err(internal)?;
    }

    sync_chapters_for_source(db, source.manga_id, manga_source_id, &source.url, force, &manga.chapters).await?;

    Ok(source.manga_id)
}

/// Reuses the already-live pg_trgm `similar()` matching (title/alt_titles fuzzy match) to
/// suggest "is this the same manga you already have?" for a title that isn't tracked
/// locally yet (e.g. a fresh search result) - used by `SearchManga.suggested_manga_id`.
pub async fn find_suggested_manga_id(db: &DatabaseConnection, title: &str) -> Result<Option<i32>, Status> {
    entity::manga::Entity::find()
        .filter(Expr::cust_with_values(
            "$1 % any(manga.alt_titles || manga.title)",
            [title.to_string()],
        ))
        .select_only()
        .column(entity::manga::Column::Id)
        .into_tuple::<i32>()
        .one(db)
        .await
        .map_err(internal)
}

pub fn index_manga(logged_in: Option<entity::user::Model>) -> Select<entity::manga::Entity> {
    join_primary_source_and_chapters(entity::manga::Entity::find())
        .column_as(entity::chapter::Column::Id.count(), "count_chapters")
        .column_as(entity::chapter::Column::Posted.max(), "last")
        .column_as(Expr::cust(NEXT_UPDATE_QUERY), "next")
        .group_by(entity::manga::Column::Id)
        .column_as(Expr::cust("null"), "progress")
        .column_as(Expr::cust("null"), "progress_ordinal")
        .apply_if(logged_in, |query, logged_in| {
            let user_id = logged_in.id;
            query
                .join(
                    JoinType::LeftJoin,
                    entity::reading::Relation::Manga
                        .def()
                        .rev()
                        .on_condition(move |_left, right| {
                            Expr::col((right, entity::reading::Column::UserId)).eq(user_id).into()
                        }),
                )
                .join(JoinType::LeftJoin, entity::reading::Relation::CanonicalChapter.def())
                .column_as(entity::reading::Column::Progress, "progress")
                .column_as(entity::canonical_chapter::Column::Ordinal, "progress_ordinal")
                .group_by(entity::reading::Column::MangaId)
                .group_by(entity::reading::Column::UserId)
                .group_by(entity::canonical_chapter::Column::Ordinal)
        })
}

#[derive(Debug, Default)]
pub struct MangaController;

#[tonic::async_trait]
impl Manga for MangaController {
    type CreateManyStream = ResponseStream;

    /// Create one manga
    async fn create(&self, request: Request<MangaRequest>) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        request
            .extensions()
            .get::<entity::user::Model>()
            .ok_or(Status::permission_denied(
                "You can only add a manga if you are logged in",
            ))?;
        let req = request.get_ref();
        let url = &req.url;

        let existing = entity::manga_source::Entity::find()
            .filter(entity::manga_source::Column::Url.eq(url.clone()))
            .one(db)
            .await
            .map_err(internal)?;

        if existing.is_some() {
            return Err(Status::already_exists("Manga with this url already exists!"));
        }

        let url = Url::parse(url).map_err(|e| Status::invalid_argument(e.to_string()))?;
        let logged_in = request.extensions().get::<entity::user::Model>().cloned();

        let manga_id = create_manga(db, url).await?;
        Ok(Response::new(get_manga_by_id(db, logged_in.as_ref(), manga_id).await?))
    }

    /// Create multiple manga
    async fn create_many(&self, request: Request<MangasRequest>) -> Result<Response<Self::CreateManyStream>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap().clone();
        let logged_in = request
            .extensions()
            .get::<entity::user::Model>()
            .ok_or(Status::permission_denied(
                "You can only add a manga if you are logged in",
            ))?
            .clone();
        let req = request.get_ref();
        let mut stream = Box::pin(tokio_stream::iter(req.urls.clone()).throttle(Duration::from_millis(200)));

        // spawn and channel are required if you want handle "disconnect" functionality
        // the `out_stream` will not be polled after client disconnect
        let (tx, rx) = mpsc::channel(128);
        tokio::spawn(async move {
            while let Some(url) = stream.next().await {
                let url = Url::parse(&url).map_err(|e| Status::invalid_argument(e.to_string()));

                let res: Result<MangaReply, Status> = match url {
                    Ok(url) => match create_manga(&db, url).await {
                        Ok(manga_id) => get_manga_by_id(&db, Some(&logged_in), manga_id).await,
                        Err(e) => Err(e),
                    },
                    Err(e) => Err(e),
                };

                info!("manga stream res: {:#?}", res);

                match tx.send(res).await {
                    Ok(_) => {
                        // item (server response) was queued to be send to client
                    }
                    Err(_item) => {
                        // output_stream was build from rx and both are dropped
                        break;
                    }
                }
            }
            println!("\tclient disconnected");
        });

        let output_stream = ReceiverStream::new(rx);
        Ok(Response::new(Box::pin(output_stream) as Self::CreateManyStream))
    }

    /// Get one manga
    async fn get(&self, request: Request<GetMangaRequest>) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize().ok();
        let req = request.get_ref();
        let manga_id = req.id;

        let (updated_at,): (NaiveDateTime,) = entity::manga::Entity::find_by_id(manga_id)
            .select_only()
            .column(entity::manga::Column::UpdatedAt)
            .into_values::<_, data::manga::Minimal>()
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("Manga not found"))?;

        let interval_ms: i64 = std::env::var("MANGA_UPDATE_INTERVAL_MS")
            .unwrap_or("3600000".to_string())
            .parse()
            .unwrap_or(3600000);

        // Check if it should be updated
        if (Utc::now().naive_utc() - chrono::Duration::milliseconds(interval_ms)) > updated_at {
            if let Some(primary_source_id) = find_primary_source_id(db, manga_id).await? {
                info!(
                    "Updating manga with id '{}' (primary source {})",
                    manga_id, primary_source_id
                );
                refresh_manga_source(db, primary_source_id, false).await?;
            }
        }

        Ok(Response::new(get_manga_by_id(db, logged_in, manga_id).await?))
    }

    /// Force update a manga - refreshes its primary source (only a primary-source refresh
    /// updates canonical manga.title/description/cover/etc). `force` also pushes a chapter
    /// reset through even if it looks like a suspicious drop (see `sync_chapters_for_source`) --
    /// for when someone has checked the source and confirmed the drop is real.
    async fn update(&self, request: Request<UpdateMangaRequest>) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize().ok();
        let req = request.get_ref();
        let manga_id = req.id;

        let primary_source_id = find_primary_source_id(db, manga_id)
            .await?
            .ok_or(Status::not_found("Manga has no primary source to refresh"))?;

        info!(
            "Updating manga with id '{}' (primary source {})",
            manga_id, primary_source_id
        );
        refresh_manga_source(db, primary_source_id, req.force).await?;

        Ok(Response::new(get_manga_by_id(db, logged_in, manga_id).await?))
    }

    /// Find or create a manga by URL
    async fn find_or_create(&self, request: Request<MangaRequest>) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in = request
            .extensions()
            .get::<entity::user::Model>()
            .ok_or(Status::permission_denied(
                "You can only add a manga if you are logged in",
            ))?
            .clone();
        let req = request.get_ref();
        let url = &req.url;

        let existing = entity::manga_source::Entity::find()
            .filter(entity::manga_source::Column::Url.eq(url.clone()))
            .one(db)
            .await
            .map_err(internal)?;

        let manga_id = if let Some(existing) = existing {
            refresh_manga_source(db, existing.id, false).await?
        } else {
            let url = Url::parse(url).map_err(|e| Status::invalid_argument(e.to_string()))?;
            create_manga(db, url).await?
        };

        Ok(Response::new(get_manga_by_id(db, Some(&logged_in), manga_id).await?))
    }

    /// Paginate manga
    async fn index(&self, request: Request<PaginateSearchQuery>) -> Result<Response<MangasReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize().ok().cloned();
        let req = request.get_ref();
        let per_page = req.per_page.unwrap_or(10).clamp(1, 50);
        let mut paginate = index_manga(logged_in);

        if let Some(search) = req.search.clone() {
            if !search.is_empty() {
                paginate = paginate.having(lucene_filter(search.into())?);
            }
        }

        if let Some(order) = req.order.clone() {
            let columns = util::order::manga::parse(&order)?;
            for (column, order) in columns {
                paginate = paginate.order_by(column, order);
            }
        } else {
            paginate = paginate.order_by(entity::manga::Column::Title, migration::Order::Asc);
        }

        let paginate = paginate.into_model::<data::manga::Full>().paginate(db, per_page);

        // Get max page and total items
        let amount = paginate.num_items_and_pages().await.map_err(internal)?;

        let max_page = if amount.number_of_pages == 0 {
            0
        } else {
            amount.number_of_pages - 1
        };

        let page = req.page.unwrap_or(0).clamp(0, max_page);

        // Get items from page
        let items = paginate.fetch_page(page).await.map_err(internal)?;

        Ok(Response::new(MangasReply {
            pagination: Some(PaginateReply {
                page,
                per_page,
                max_page,
                total: amount.number_of_items,
            }),
            items: attach_sources(db, items).await?,
        }))
    }

    async fn similar(&self, request: Request<SimilarMangaRequest>) -> Result<Response<MangasReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize().ok().cloned();

        let id = request.get_ref().id;
        let (manga_title, alt_titles): (String, Vec<String>) = entity::manga::Entity::find_by_id(id)
            .select_only()
            .column(entity::manga::Column::Title)
            .column(entity::manga::Column::AltTitles)
            .into_tuple()
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("Manga not found"))?;

        let title_matches = alt_titles
            .into_iter()
            .map(|alt_title| Expr::cust_with_values("$1 % any(manga.alt_titles || manga.title)", [alt_title]))
            .fold(
                Expr::cust_with_values("$1 % any(manga.alt_titles || manga.title)", [manga_title]),
                |expr, alt_title_expr| expr.or(alt_title_expr),
            );

        let similar = index_manga(logged_in)
            .filter(entity::manga::Column::Id.ne(id).and(title_matches))
            .into_model::<data::manga::Full>()
            .all(db)
            .await
            .map_err(internal)?;

        Ok(Response::new(MangasReply {
            pagination: None,
            items: attach_sources(db, similar).await?,
        }))
    }

    /// Add a new (secondary, by default) source to an existing manga.
    async fn add_source(&self, request: Request<AddSourceRequest>) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in = request
            .extensions()
            .get::<entity::user::Model>()
            .ok_or(Status::permission_denied(
                "You can only add a source if you are logged in",
            ))?
            .clone();
        let req = request.get_ref();

        let url = Url::parse(&req.url).map_err(|e| Status::invalid_argument(e.to_string()))?;
        add_manga_source(db, req.manga_id, url).await?;

        Ok(Response::new(
            get_manga_by_id(db, Some(&logged_in), req.manga_id).await?,
        ))
    }

    /// Remove a source from a manga. If it's the primary source and another source
    /// still exists, that other source is auto-promoted to primary first so canonical
    /// manga metadata is never left without a primary source backing it.
    async fn remove_source(&self, request: Request<RemoveSourceRequest>) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in = request
            .extensions()
            .get::<entity::user::Model>()
            .ok_or(Status::permission_denied(
                "You can only remove a source if you are logged in",
            ))?
            .clone();
        let req = request.get_ref();

        let source = entity::manga_source::Entity::find_by_id(req.manga_source_id)
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("Manga source not found"))?;

        if source.is_primary {
            promote_replacement_primary(db, source.manga_id, source.id).await?;
        }

        let manga_id = source.manga_id;
        entity::manga_source::Entity::delete_by_id(req.manga_source_id)
            .exec(db)
            .await
            .map_err(internal)?;

        let remaining_sources = entity::manga_source::Entity::find()
            .filter(entity::manga_source::Column::MangaId.eq(manga_id))
            .count(db)
            .await
            .map_err(internal)?;

        // A manga with zero sources can never be scraped/refreshed again - rather than
        // leaving a dead, sourceless husk lying around (which would then need filtering out
        // of every listing/similar/search query, forever), just delete it outright. Cascades
        // away its reading/canonical_chapter rows too. This intentionally does NOT reserve
        // room for "a manga that legitimately has zero sources" (e.g. a future external-index
        // import) - today there's no such case, and if one's ever added, it'd be a deliberate
        // exception to this rule, not the other way around.
        if remaining_sources == 0 {
            entity::manga::Entity::delete_by_id(manga_id)
                .exec(db)
                .await
                .map_err(internal)?;

            info!("Deleted manga {} - its last source was just removed", manga_id);

            return Err(Status::not_found(format!(
                "Manga {manga_id} was deleted - that was its last remaining source"
            )));
        }

        Ok(Response::new(get_manga_by_id(db, Some(&logged_in), manga_id).await?))
    }

    /// Change which source is considered primary for a manga (the only source whose
    /// refreshes update canonical manga.title/description/cover/etc, and whose chapters
    /// back top-level count_chapters/last/next).
    async fn set_primary_source(
        &self,
        request: Request<SetPrimarySourceRequest>,
    ) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in = request
            .extensions()
            .get::<entity::user::Model>()
            .ok_or(Status::permission_denied(
                "You can only change the primary source if you are logged in",
            ))?
            .clone();
        let req = request.get_ref();

        let source = entity::manga_source::Entity::find_by_id(req.manga_source_id)
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("Manga source not found"))?;

        entity::manga_source::Entity::update_many()
            .col_expr(entity::manga_source::Column::IsPrimary, Expr::value(false))
            .filter(entity::manga_source::Column::MangaId.eq(source.manga_id))
            .exec(db)
            .await
            .map_err(internal)?;

        entity::manga_source::ActiveModel {
            id: Set(source.id),
            is_primary: Set(true),
            ..Default::default()
        }
        .update(db)
        .await
        .map_err(internal)?;

        // Refresh the newly-primary source's metadata immediately, rather than leaving
        // canonical title/description/cover stale until the next scheduled scrape - the
        // primary-source flag has already flipped by this point regardless of whether this
        // refresh succeeds, so a failure here (e.g. the new primary is briefly unreachable)
        // is logged, not fatal to the RPC.
        if let Err(e) = refresh_manga_source(db, source.id, false).await {
            warn!(
                "Failed to immediately refresh newly-primary source {} for manga {}: {:#?}",
                source.id, source.manga_id, e
            );
        }

        Ok(Response::new(
            get_manga_by_id(db, Some(&logged_in), source.manga_id).await?,
        ))
    }

    /// Admin-only. Folds every source of `source_manga_id` into `target_manga_id` -
    /// reading progress reconciled (see merge_reading_progress), chapters relinked onto
    /// the target's canonical rows - then deletes `source_manga_id`. The one safe way to
    /// fix a duplicate manga created by adding the same title from a different source
    /// (FindOrCreate only dedupes by exact URL).
    async fn merge_manga(&self, request: Request<MergeMangaRequest>) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?.clone();
        if !logged_in.has_permission(UserPermissions::ADMIN) {
            return Err(Status::permission_denied("Admin permission required"));
        }
        let req = request.get_ref();

        if req.source_manga_id == req.target_manga_id {
            return Err(Status::invalid_argument("Cannot merge a manga into itself"));
        }

        let txn = db.begin().await.map_err(internal)?;
        merge_manga_sources(&txn, req.source_manga_id, req.target_manga_id).await?;
        txn.commit().await.map_err(internal)?;

        Ok(Response::new(
            get_manga_by_id(db, Some(&logged_in), req.target_manga_id).await?,
        ))
    }

    /// Admin-only. Moves a single source onto a different manga (see move_source_to_manga).
    /// If that empties out its old manga, that manga's reading progress is folded into the
    /// target and it's deleted, same as a full MergeManga would do for it.
    async fn move_source(&self, request: Request<MoveSourceRequest>) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?.clone();
        if !logged_in.has_permission(UserPermissions::ADMIN) {
            return Err(Status::permission_denied("Admin permission required"));
        }
        let req = request.get_ref();

        let txn = db.begin().await.map_err(internal)?;
        move_source_to_manga(&txn, req.manga_source_id, req.target_manga_id).await?;
        txn.commit().await.map_err(internal)?;

        Ok(Response::new(
            get_manga_by_id(db, Some(&logged_in), req.target_manga_id).await?,
        ))
    }

    /// Kicks off (or resumes) a throttled background walk downloading every
    /// not-yet-cached page of every chapter of that source.
    async fn backfill_images(&self, request: Request<BackfillImagesRequest>) -> Result<Response<Empty>, Status> {
        let db = request.db()?.clone();
        let manga_source_id = request.get_ref().manga_source_id;

        backfill::start_backfill(db, manga_source_id);

        Ok(Response::new(Empty::default()))
    }

    /// See BackfillImages. Cheap `GROUP BY status` progress count for the client
    /// to poll on its own schedule.
    async fn get_backfill_status(
        &self,
        request: Request<GetBackfillStatusRequest>,
    ) -> Result<Response<BackfillStatusReply>, Status> {
        let db = request.db()?;
        let manga_source_id = request.get_ref().manga_source_id;

        let (images_downloaded, images_total) = backfill::backfill_status(db, manga_source_id).await?;

        Ok(Response::new(BackfillStatusReply {
            images_downloaded,
            images_total,
        }))
    }
}

async fn find_primary_source_id(db: &DatabaseConnection, manga_id: i32) -> Result<Option<i32>, Status> {
    entity::manga_source::Entity::find()
        .filter(entity::manga_source::Column::MangaId.eq(manga_id))
        .filter(entity::manga_source::Column::IsPrimary.eq(true))
        .select_only()
        .column(entity::manga_source::Column::Id)
        .into_tuple::<i32>()
        .one(db)
        .await
        .map_err(internal)
}

crate::export_service!(MangaServer, MangaController);

#[cfg(test)]
mod matching_heuristic_tests {
    use manga_parser::model::Chapter;
    use manga_parser::Url;
    use sea_orm::Database;

    use super::*;

    // `chapter.url` is globally unique across all sources (unchanged from before Phase 1),
    // so distinct sources always need distinct urls - just like real different sites never
    // share a literal URL string.
    fn chapter(source: &str, number: f32) -> Chapter {
        Chapter {
            url: Url::parse(&format!("https://{source}.test/c{number}")).unwrap(),
            number,
            title: format!("Chapter {number}"),
            date: None,
        }
    }

    /// Temporary validation harness for the Phase 1b matching heuristic - requires
    /// DATABASE_URL to point at a throwaway, already-migrated test database (never the
    /// real one). Run with:
    ///   DATABASE_URL=postgres://... cargo test -p rumgap matching_heuristic -- --ignored --nocapture
    #[ignore]
    #[tokio::test]
    async fn convergence_and_ambiguity() {
        let db_url = std::env::var("DATABASE_URL").expect("set DATABASE_URL to a throwaway test DB");
        let db = Database::connect(db_url).await.unwrap();

        let manga = entity::manga::ActiveModel {
            title: Set("Test Manga".into()),
            description: Set("".into()),
            is_ongoing: Set(true),
            authors: Set(vec![]),
            alt_titles: Set(vec![]),
            genres: Set(vec![]),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();

        let source_a = entity::manga_source::ActiveModel {
            manga_id: Set(manga.id),
            url: Set("https://a.test/manga".into()),
            hostname: Set("a.test".into()),
            is_primary: Set(true),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();

        // Source A: chapters 1, 2, 3 - all brand new, each gets its own solo canonical row.
        sync_chapters_for_source(
            &db,
            manga.id,
            source_a.id,
            &source_a.url,
            false,
            &[chapter("a", 1.0), chapter("a", 2.0), chapter("a", 3.0)],
        )
        .await
        .unwrap();

        let canonical_count = entity::canonical_chapter::Entity::find()
            .filter(entity::canonical_chapter::Column::MangaId.eq(manga.id))
            .count(&db)
            .await
            .unwrap();
        assert_eq!(canonical_count, 3, "expected 3 solo canonical rows after source A");

        let source_b = entity::manga_source::ActiveModel {
            manga_id: Set(manga.id),
            url: Set("https://b.test/manga".into()),
            hostname: Set("b.test".into()),
            is_primary: Set(false),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();

        // Source B: chapters 1, 2 (should converge onto A's existing canonical rows) and a
        // new chapter 4 (should create a 4th canonical row).
        sync_chapters_for_source(
            &db,
            manga.id,
            source_b.id,
            &source_b.url,
            false,
            &[chapter("b", 1.0), chapter("b", 2.0), chapter("b", 4.0)],
        )
        .await
        .unwrap();

        let canonical_count = entity::canonical_chapter::Entity::find()
            .filter(entity::canonical_chapter::Column::MangaId.eq(manga.id))
            .count(&db)
            .await
            .unwrap();
        assert_eq!(
            canonical_count, 4,
            "expected only 1 new canonical row (ordinal 4) after source B converges on 1/2"
        );

        let chapters_a = entity::chapter::Entity::find()
            .filter(entity::chapter::Column::MangaSourceId.eq(source_a.id))
            .all(&db)
            .await
            .unwrap();
        let chapters_b = entity::chapter::Entity::find()
            .filter(entity::chapter::Column::MangaSourceId.eq(source_b.id))
            .all(&db)
            .await
            .unwrap();

        let chapter_a1 = chapters_a.iter().find(|c| c.number == 1.0).unwrap();
        let chapter_b1 = chapters_b.iter().find(|c| c.number == 1.0).unwrap();
        assert_eq!(
            chapter_a1.canonical_chapter_id, chapter_b1.canonical_chapter_id,
            "source A's and source B's chapter 1 should converge on the same canonical row"
        );
        assert!(chapter_a1.canonical_chapter_id.is_some());

        let chapter_a2 = chapters_a.iter().find(|c| c.number == 2.0).unwrap();
        let chapter_b2 = chapters_b.iter().find(|c| c.number == 2.0).unwrap();
        assert_eq!(chapter_a2.canonical_chapter_id, chapter_b2.canonical_chapter_id);

        let chapter_b4 = chapters_b.iter().find(|c| c.number == 4.0).unwrap();
        assert!(chapter_b4.canonical_chapter_id.is_some());
        assert_ne!(chapter_b4.canonical_chapter_id, chapter_a1.canonical_chapter_id);

        // Source C: two chapters in the *same* scrape both claim ordinal 5 - ambiguous,
        // should leave the second one unlinked rather than guessing, and only ever create
        // one canonical row at ordinal 5 (the unique(manga_id, ordinal) constraint couldn't
        // allow a second one anyway).
        let source_c = entity::manga_source::ActiveModel {
            manga_id: Set(manga.id),
            url: Set("https://c.test/manga".into()),
            hostname: Set("c.test".into()),
            is_primary: Set(false),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();

        let dup_a = chapter("c-5a", 5.0);
        let dup_b = chapter("c-5b", 5.0);

        sync_chapters_for_source(&db, manga.id, source_c.id, &source_c.url, false, &[dup_a, dup_b])
            .await
            .unwrap();

        let chapters_c = entity::chapter::Entity::find()
            .filter(entity::chapter::Column::MangaSourceId.eq(source_c.id))
            .all(&db)
            .await
            .unwrap();
        assert_eq!(chapters_c.len(), 2);
        let linked_count = chapters_c.iter().filter(|c| c.canonical_chapter_id.is_some()).count();
        assert_eq!(
            linked_count, 1,
            "exactly one of the two same-ordinal chapters should be linked"
        );

        let canonical_count_5 = entity::canonical_chapter::Entity::find()
            .filter(entity::canonical_chapter::Column::MangaId.eq(manga.id))
            .filter(entity::canonical_chapter::Column::Ordinal.eq(sea_orm::prelude::Decimal::new(5000, 3)))
            .count(&db)
            .await
            .unwrap();
        assert_eq!(
            canonical_count_5, 1,
            "only one canonical row should ever exist at ordinal 5"
        );

        // Cleanup
        entity::manga::Entity::delete_by_id(manga.id).exec(&db).await.unwrap();
    }

    async fn make_manga(db: &sea_orm::DatabaseConnection, title: &str) -> entity::manga::Model {
        entity::manga::ActiveModel {
            title: Set(title.into()),
            description: Set("".into()),
            is_ongoing: Set(true),
            authors: Set(vec![]),
            alt_titles: Set(vec![]),
            genres: Set(vec![]),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap()
    }

    async fn make_source(
        db: &sea_orm::DatabaseConnection,
        manga_id: i32,
        hostname: &str,
        is_primary: bool,
    ) -> entity::manga_source::Model {
        entity::manga_source::ActiveModel {
            manga_id: Set(manga_id),
            url: Set(format!("https://{hostname}/manga")),
            hostname: Set(hostname.into()),
            is_primary: Set(is_primary),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap()
    }

    async fn make_user(db: &sea_orm::DatabaseConnection, username: &str) -> entity::user::Model {
        entity::user::ActiveModel {
            username: Set(username.into()),
            email: Set(format!("{username}@test.local")),
            password_hash: Set("".into()),
            preferred_hostnames: Set(vec![]),
            device_ids: Set(vec![]),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap()
    }

    /// Sets up an existing (user, manga) reading row linked to `canonical_chapter_id`, via
    /// the same two-step path the real Reading.Create + Reading.Update(chapter_id) RPCs use.
    async fn make_reading(db: &sea_orm::DatabaseConnection, user_id: i32, manga_id: i32, canonical_chapter_id: i32) {
        entity::reading::ActiveModel {
            user_id: Set(user_id),
            manga_id: Set(manga_id),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();

        sync_reading_progress(db, user_id, manga_id, canonical_chapter_id).await.unwrap();
    }

    /// Requires DATABASE_URL to point at a throwaway, already-migrated test database (never
    /// the real one). Run with:
    ///   DATABASE_URL=postgres://... cargo test -p rumgap move_source_relinks -- --ignored --nocapture
    #[ignore]
    #[tokio::test]
    async fn move_source_relinks_canonical_chapters_onto_target() {
        let db_url = std::env::var("DATABASE_URL").expect("set DATABASE_URL to a throwaway test DB");
        let db = Database::connect(db_url).await.unwrap();

        let manga_a = make_manga(&db, "Manga A").await;
        let source_a = make_source(&db, manga_a.id, "move-a.test", true).await;
        sync_chapters_for_source(
            &db,
            manga_a.id,
            source_a.id,
            &source_a.url,
            false,
            &[chapter("move-a-1", 1.0), chapter("move-a-2", 2.0), chapter("move-a-3", 3.0)],
        )
        .await
        .unwrap();

        let manga_b = make_manga(&db, "Manga B").await;
        let source_b = make_source(&db, manga_b.id, "move-b.test", true).await;
        sync_chapters_for_source(
            &db,
            manga_b.id,
            source_b.id,
            &source_b.url,
            false,
            &[chapter("move-b-2", 2.0), chapter("move-b-3", 3.0), chapter("move-b-4", 4.0)],
        )
        .await
        .unwrap();

        // Move source_a (manga_a's only source) onto manga_b - manga_a should end up
        // sourceless and get deleted, and source_a's chapters should relink onto manga_b's
        // canonical rows: converging with the existing ones at ordinals 2/3, and creating a
        // fresh one at ordinal 1 (which manga_b never had).
        move_source_to_manga(&db, source_a.id, manga_b.id).await.unwrap();

        assert!(
            entity::manga::Entity::find_by_id(manga_a.id).one(&db).await.unwrap().is_none(),
            "manga_a should be deleted once its only source moved away"
        );

        let moved_source = entity::manga_source::Entity::find_by_id(source_a.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(moved_source.manga_id, manga_b.id);
        assert!(!moved_source.is_primary, "moved source must not steal manga_b's primary flag");

        let chapters_a = entity::chapter::Entity::find()
            .filter(entity::chapter::Column::MangaSourceId.eq(source_a.id))
            .all(&db)
            .await
            .unwrap();
        let chapters_b = entity::chapter::Entity::find()
            .filter(entity::chapter::Column::MangaSourceId.eq(source_b.id))
            .all(&db)
            .await
            .unwrap();

        let moved_2 = chapters_a.iter().find(|c| c.number == 2.0).unwrap();
        let target_2 = chapters_b.iter().find(|c| c.number == 2.0).unwrap();
        assert_eq!(
            moved_2.canonical_chapter_id, target_2.canonical_chapter_id,
            "moved chapter 2 should converge onto manga_b's existing ordinal-2 canonical row"
        );

        let canonical_2_count = entity::canonical_chapter::Entity::find()
            .filter(entity::canonical_chapter::Column::MangaId.eq(manga_b.id))
            .filter(entity::canonical_chapter::Column::Ordinal.eq(sea_orm::prelude::Decimal::new(2000, 3)))
            .count(&db)
            .await
            .unwrap();
        assert_eq!(canonical_2_count, 1, "must reuse the existing ordinal-2 row, not duplicate it");

        let moved_1 = chapters_a.iter().find(|c| c.number == 1.0).unwrap();
        let canonical_1 = entity::canonical_chapter::Entity::find_by_id(moved_1.canonical_chapter_id.unwrap())
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(canonical_1.manga_id, manga_b.id, "ordinal 1 (new to manga_b) must be created under manga_b");

        // Cleanup
        entity::manga::Entity::delete_by_id(manga_b.id).exec(&db).await.unwrap();
    }

    /// Requires DATABASE_URL to point at a throwaway, already-migrated test database (never
    /// the real one). Run with:
    ///   DATABASE_URL=postgres://... cargo test -p rumgap merge_manga_reconciles -- --ignored --nocapture
    #[ignore]
    #[tokio::test]
    async fn merge_manga_reconciles_reading_progress() {
        let db_url = std::env::var("DATABASE_URL").expect("set DATABASE_URL to a throwaway test DB");
        let db = Database::connect(db_url).await.unwrap();

        let manga_a = make_manga(&db, "Merge A").await;
        let source_a = make_source(&db, manga_a.id, "merge-a.test", true).await;
        sync_chapters_for_source(
            &db,
            manga_a.id,
            source_a.id,
            &source_a.url,
            false,
            &[chapter("merge-a-1", 1.0), chapter("merge-a-2", 2.0), chapter("merge-a-3", 3.0)],
        )
        .await
        .unwrap();

        let manga_b = make_manga(&db, "Merge B").await;
        let source_b = make_source(&db, manga_b.id, "merge-b.test", true).await;
        sync_chapters_for_source(
            &db,
            manga_b.id,
            source_b.id,
            &source_b.url,
            false,
            &[chapter("merge-b-1", 1.0), chapter("merge-b-2", 2.0), chapter("merge-b-3", 3.0)],
        )
        .await
        .unwrap();

        let canonical_a = |ordinal: f32| {
            entity::canonical_chapter::Entity::find()
                .filter(entity::canonical_chapter::Column::MangaId.eq(manga_a.id))
                .filter(entity::canonical_chapter::Column::Ordinal.eq(sea_orm::prelude::Decimal::from_f32_retain(ordinal).unwrap()))
                .one(&db)
        };
        let canonical_b = |ordinal: f32| {
            entity::canonical_chapter::Entity::find()
                .filter(entity::canonical_chapter::Column::MangaId.eq(manga_b.id))
                .filter(entity::canonical_chapter::Column::Ordinal.eq(sea_orm::prelude::Decimal::from_f32_retain(ordinal).unwrap()))
                .one(&db)
        };

        let user1 = make_user(&db, "merge-user1").await; // further along on A - A should win
        let user2 = make_user(&db, "merge-user2").await; // only ever read B - untouched
        let user3 = make_user(&db, "merge-user3").await; // further along on B - B should win

        make_reading(&db, user1.id, manga_a.id, canonical_a(3.0).await.unwrap().unwrap().id).await;
        make_reading(&db, user1.id, manga_b.id, canonical_b(1.0).await.unwrap().unwrap().id).await;

        make_reading(&db, user2.id, manga_b.id, canonical_b(2.0).await.unwrap().unwrap().id).await;

        make_reading(&db, user3.id, manga_a.id, canonical_a(1.0).await.unwrap().unwrap().id).await;
        make_reading(&db, user3.id, manga_b.id, canonical_b(2.0).await.unwrap().unwrap().id).await;

        merge_manga_sources(&db, manga_a.id, manga_b.id).await.unwrap();

        assert!(
            entity::manga::Entity::find_by_id(manga_a.id).one(&db).await.unwrap().is_none(),
            "manga_a should be deleted once fully merged into manga_b"
        );

        for user in [&user1, &user2, &user3] {
            assert!(
                entity::reading::Entity::find_by_id((user.id, manga_a.id))
                    .one(&db)
                    .await
                    .unwrap()
                    .is_none(),
                "no reading row should survive for the deleted manga_a"
            );
        }

        let user1_reading = entity::reading::Entity::find_by_id((user1.id, manga_b.id))
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(user1_reading.progress, 3, "user1 was further along on A (3 > 1) - A's progress should win");
        let user1_canonical = entity::canonical_chapter::Entity::find_by_id(user1_reading.last_canonical_chapter_id.unwrap())
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(user1_canonical.manga_id, manga_b.id);
        assert_eq!(user1_canonical.ordinal, sea_orm::prelude::Decimal::new(3000, 3));

        let user2_reading = entity::reading::Entity::find_by_id((user2.id, manga_b.id))
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(user2_reading.progress, 2, "user2 never had progress on A - manga_b's row must be untouched");

        let user3_reading = entity::reading::Entity::find_by_id((user3.id, manga_b.id))
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(user3_reading.progress, 2, "user3 was further along on B (2 > 1) - B's progress should win");

        // Cleanup
        entity::manga::Entity::delete_by_id(manga_b.id).exec(&db).await.unwrap();
        for user in [user1, user2, user3] {
            entity::user::Entity::delete_by_id(user.id).exec(&db).await.unwrap();
        }
    }
}
