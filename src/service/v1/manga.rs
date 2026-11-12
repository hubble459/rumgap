use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::time::Duration;

use chrono::{NaiveDateTime, Utc};
use futures::Stream;
use manga_parser::scraper::MangaScraper;
use manga_parser::Url;
use migration::{Expr, ExprTrait, JoinType, OnConflict};
use sea_orm::ActiveValue::Set;
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
    AddSourceRequest, BackfillImagesRequest, DeleteMangaRequest, DismissMangaRequest, BackfillStatusReply, Empty, GetBackfillStatusRequest, GetMangaRequest,
    MangaReply, MangaRequest, MangaSourceReply, MangasReply, MangasRequest, MergeMangaRequest, MoveSourceRequest,
    PaginateQuery, PaginateReply, PaginateSearchQuery, RemoveSourceRequest, SelectSourceRequest, SetNsfwRequest, SetPrimarySourceRequest, SimilarMangaRequest,
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

/// The one source a user sees a manga through: the one they last picked for it
/// (`reading.preferred_source_id`), else the first of their preferred hostnames the manga
/// has, else the primary. `$1` is the user's id, `$2` their preferred hostnames.
const DISPLAY_SOURCE_QUERY: &str = r#"manga_source.id = (
    SELECT s.id FROM manga_source s
    LEFT JOIN reading r ON r.manga_id = s.manga_id AND r.user_id = $1
    WHERE s.manga_id = manga.id
    ORDER BY (s.id = r.preferred_source_id) DESC NULLS LAST,
             array_position($2::text[], s.hostname::text) NULLS LAST,
             s.is_primary DESC,
             s.id
    LIMIT 1
)"#;

/// The selected source's cover, or the primary's while the selected one has none yet.
const COVER_SOURCE_QUERY: &str = r#"CASE WHEN manga_source.cover_source_url IS NOT NULL THEN manga_source.id ELSE (
    SELECT p.id FROM manga_source p
    WHERE p.manga_id = manga.id AND p.is_primary AND p.cover_source_url IS NOT NULL
    LIMIT 1
) END"#;

/// Everything `data::manga::Full` needs, for `query`'s manga as `logged_in` sees them:
/// details, chapter counts and dates come from the source picked by `DISPLAY_SOURCE_QUERY`.
fn select_manga(
    query: Select<entity::manga::Entity>,
    logged_in: Option<&entity::user::Model>,
) -> Select<entity::manga::Entity> {
    use entity::manga_source::Column as Source;

    let user_id = logged_in.map_or(-1, |user| user.id);
    let hostnames = logged_in
        .map(|user| user.preferred_hostnames.clone())
        .unwrap_or_default();
    query
        .join(
            JoinType::LeftJoin,
            entity::manga::Relation::MangaSource
                .def()
                .on_condition(move |_left, _right| {
                    Expr::cust_with_values::<_, sea_orm::Value, _>(
                        DISPLAY_SOURCE_QUERY,
                        [user_id.into(), hostnames.clone().into()],
                    )
                    .into()
                }),
        )
        .join(JoinType::LeftJoin, entity::manga_source::Relation::Chapter.def())
        .column_as(Source::Id, "selected_source_id")
        .column_as(Source::Title, "title")
        .column_as(Source::Description, "description")
        .column_as(Source::Status, "status")
        .column_as(Source::IsOngoing, "is_ongoing")
        .column_as(Source::Genres, "genres")
        .column_as(Source::Authors, "authors")
        .column_as(Source::AltTitles, "alt_titles")
        .column_as(Expr::cust(COVER_SOURCE_QUERY), "cover_source_id")
        .column_as(entity::chapter::Column::Id.count(), "count_chapters")
        .column_as(entity::chapter::Column::Posted.max(), "last")
        .column_as(Expr::cust(NEXT_UPDATE_QUERY), "next")
        .group_by(entity::manga::Column::Id)
        .group_by(Source::Id)
        .column_as(Expr::cust("null"), "progress")
        .column_as(Expr::cust("null"), "progress_ordinal")
        .apply_if(logged_in.map(|user| user.id), |query, user_id| {
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
                .group_by(entity::reading::Column::UserId)
                .group_by(entity::reading::Column::MangaId)
                .group_by(entity::canonical_chapter::Column::Ordinal)
        })
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

    let source_ids: Vec<i32> = sources.iter().map(|source| source.id).collect();
    let fully_stored = fully_stored_source_ids(db, source_ids.clone()).await?;
    let chapter_stats = source_chapter_stats(db, source_ids).await?;

    let mut map: HashMap<i32, Vec<MangaSourceReply>> = HashMap::new();
    for source in sources {
        let manga_id = source.manga_id;
        let mut reply: MangaSourceReply = source.into();
        reply.fully_stored = fully_stored.contains(&reply.id);
        if let Some((count, last)) = chapter_stats.get(&reply.id) {
            reply.count_chapters = *count;
            reply.last = last.map(|date| date.timestamp_millis());
        }
        map.entry(manga_id).or_default().push(reply);
    }
    Ok(map)
}

/// Chapter count and newest chapter's posted date per source.
async fn source_chapter_stats(
    db: &DatabaseConnection,
    source_ids: Vec<i32>,
) -> Result<HashMap<i32, (i64, Option<sea_orm::prelude::DateTimeWithTimeZone>)>, Status> {
    if source_ids.is_empty() {
        return Ok(HashMap::new());
    }

    entity::chapter::Entity::find()
        .select_only()
        .column(entity::chapter::Column::MangaSourceId)
        .column_as(entity::chapter::Column::Id.count(), "count")
        .column_as(entity::chapter::Column::Posted.max(), "last")
        .filter(entity::chapter::Column::MangaSourceId.is_in(source_ids))
        .group_by(entity::chapter::Column::MangaSourceId)
        .into_tuple::<(i32, i64, Option<sea_orm::prelude::DateTimeWithTimeZone>)>()
        .all(db)
        .await
        .map_err(internal)
        .map(|rows| rows.into_iter().map(|(id, count, last)| (id, (count, last))).collect())
}

/// The subset of `source_ids` where every chapter has `chapter_image` rows and all of them are
/// `done`. A chapter whose pages were never listed counts as not stored.
pub async fn fully_stored_source_ids(db: &DatabaseConnection, source_ids: Vec<i32>) -> Result<HashSet<i32>, Status> {
    if source_ids.is_empty() {
        return Ok(HashSet::new());
    }

    let rows = db
        .query_all_raw(sea_orm::Statement::from_sql_and_values(
            db.get_database_backend(),
            r#"
            SELECT chapter.manga_source_id
            FROM chapter
            LEFT JOIN chapter_image ON chapter_image.chapter_id = chapter.id
            WHERE chapter.manga_source_id = ANY($1)
            GROUP BY chapter.manga_source_id
            HAVING bool_and(chapter_image.status IS NOT DISTINCT FROM 'done')
            "#,
            [source_ids.into()],
        ))
        .await
        .map_err(internal)?;

    rows.iter()
        .map(|row| row.try_get_by_index::<i32>(0).map_err(internal))
        .collect()
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
pub async fn get_manga_by_id(
    db: &DatabaseConnection,
    logged_in: Option<&entity::user::Model>,
    manga_id: i32,
) -> Result<MangaReply, Status> {
    let manga = select_manga(entity::manga::Entity::find_by_id(manga_id), logged_in)
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
    if chapters.is_empty() {
        return Ok(());
    }

    fn ordinal_of(chapter: &entity::chapter::Model) -> sea_orm::prelude::Decimal {
        sea_orm::prelude::Decimal::from_f32_retain(chapter.number)
            .unwrap_or_default()
            .round_dp(3)
    }

    let mut unique_ordinals: Vec<sea_orm::prelude::Decimal> = chapters.iter().map(ordinal_of).collect();
    unique_ordinals.sort();
    unique_ordinals.dedup();

    // Batch-fetch every canonical_chapter row this scrape could possibly need, in one
    // query, instead of a per-chapter SELECT.
    let mut ordinal_to_id: HashMap<sea_orm::prelude::Decimal, i32> = entity::canonical_chapter::Entity::find()
        .filter(entity::canonical_chapter::Column::MangaId.eq(manga_id))
        .filter(entity::canonical_chapter::Column::Ordinal.is_in(unique_ordinals.clone()))
        .all(db)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|row| (row.ordinal, row.id))
        .collect();

    let missing: Vec<sea_orm::prelude::Decimal> = unique_ordinals
        .into_iter()
        .filter(|ordinal| !ordinal_to_id.contains_key(ordinal))
        .collect();

    if !missing.is_empty() {
        // ON CONFLICT DO NOTHING + RETURNING: several chapters in this same scrape can
        // share an ordinal (e.g. two scanlation groups both releasing "chapter 5"), so the
        // unique(manga_id, ordinal) constraint means only the first of them actually
        // inserts here - the rest resolve via `ordinal_to_id` below, same as before.
        let new_rows = missing.iter().map(|ordinal| entity::canonical_chapter::ActiveModel {
            manga_id: Set(manga_id),
            ordinal: Set(*ordinal),
            ..Default::default()
        });

        let inserted = entity::canonical_chapter::Entity::insert_many(new_rows)
            .on_conflict(
                OnConflict::columns([
                    entity::canonical_chapter::Column::MangaId,
                    entity::canonical_chapter::Column::Ordinal,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_with_returning(db)
            .await
            .map_err(internal)?;

        ordinal_to_id.extend(inserted.into_iter().map(|row| (row.ordinal, row.id)));

        // Rare race: a concurrent writer inserted the same (manga_id, ordinal) between our
        // SELECT and this INSERT, so ON CONFLICT DO NOTHING skipped it here too - one
        // follow-up query picks up whatever's still missing.
        let still_missing: Vec<sea_orm::prelude::Decimal> = missing
            .into_iter()
            .filter(|ordinal| !ordinal_to_id.contains_key(ordinal))
            .collect();

        if !still_missing.is_empty() {
            let rows = entity::canonical_chapter::Entity::find()
                .filter(entity::canonical_chapter::Column::MangaId.eq(manga_id))
                .filter(entity::canonical_chapter::Column::Ordinal.is_in(still_missing))
                .all(db)
                .await
                .map_err(internal)?;
            ordinal_to_id.extend(rows.into_iter().map(|row| (row.ordinal, row.id)));
        }
    }

    let chapter_ids: Vec<i32> = chapters.iter().map(|chapter| chapter.id).collect();
    let canonical_ids: Vec<i32> = chapters
        .iter()
        .map(|chapter| ordinal_to_id[&ordinal_of(chapter)])
        .collect();

    // Bulk-apply every chapter -> canonical_chapter link in one round trip instead of one
    // UPDATE per chapter.
    let bulk_update = sea_orm::Statement::from_sql_and_values(
        db.get_database_backend(),
        r#"
        UPDATE chapter AS c
        SET canonical_chapter_id = data.canonical_chapter_id
        FROM (SELECT * FROM UNNEST($1::int[], $2::int[]) AS t(id, canonical_chapter_id)) AS data
        WHERE c.id = data.id
        "#,
        [chapter_ids.into(), canonical_ids.into()],
    );
    db.execute_raw(bulk_update).await.map_err(internal)?;

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
/// away, promotes it to primary - so a manga always has a primary source to fall back on.
/// No-op if that was the manga's only source.
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

    // Reading history of this source's chapters moves along with them.
    db.execute_raw(sea_orm::Statement::from_sql_and_values(
        db.get_database_backend(),
        "UPDATE read_event SET manga_id = $1 WHERE chapter_id IN (SELECT id FROM chapter WHERE manga_source_id = $2)",
        [target_manga_id.into(), manga_source_id.into()],
    ))
    .await
    .map_err(internal)?;

    let remaining_sources = entity::manga_source::Entity::find()
        .filter(entity::manga_source::Column::MangaId.eq(old_manga_id))
        .count(db)
        .await
        .map_err(internal)?;

    if remaining_sources == 0 {
        merge_reading_progress(db, old_manga_id, target_manga_id).await?;

        // Whatever is left on the husk (progress events, comments) belongs to the target now.
        for table in ["read_event", "comment"] {
            db.execute_raw(sea_orm::Statement::from_sql_and_values(
                db.get_database_backend(),
                format!("UPDATE {table} SET manga_id = $1 WHERE manga_id = $2"),
                [target_manga_id.into(), old_manga_id.into()],
            ))
            .await
            .map_err(internal)?;
        }

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

/// Sets the details `scraped` found on `source` (a new source when its `id` is `NotSet`).
/// When the scraped cover differs from the stored one, the stored file is deleted and the
/// cover is downloaded again on its next request. A scrape without a cover keeps the old one.
async fn apply_scraped_details(
    mut source: entity::manga_source::ActiveModel,
    current: Option<&entity::manga_source::Model>,
    scraped: &manga_parser::model::Manga,
) -> entity::manga_source::ActiveModel {
    source.title = Set(scraped.title.clone());
    source.description = Set(scraped.description.clone());
    source.is_ongoing = Set(scraped.is_ongoing);
    source.authors = Set(scraped.authors.clone());
    source.alt_titles = Set(scraped.alternative_titles.clone());
    source.genres = Set(scraped.genres.clone());
    if let Some(status) = &scraped.status {
        source.status = Set(status.clone());
    }
    source.details_updated_at = Set(Some(Utc::now().naive_utc()));

    // cover_url is optional in manga_parser's model (unlike title/description, which fail the
    // whole scrape when missing), so a successful scrape without one must not wipe a cover.
    if let Some(cover_url) = scraped.cover_url.as_ref().map(|url| url.to_string()) {
        if current.and_then(|current| current.cover_source_url.as_deref()) != Some(cover_url.as_str()) {
            if let Some(old_key) = current.and_then(|current| current.cover_storage_key.as_ref()) {
                if let Err(e) = crate::IMAGE_STORE.delete(old_key).await {
                    warn!("Failed to delete stale cover {}: {}", old_key, e);
                }
            }
            source.cover_status = Set("pending".to_string());
            source.cover_storage_key = Set(None);
            source.cover_content_type = Set(None);
            source.cover_checksum = Set(None);
            source.cover_attempts = Set(0);
            source.cover_source_url = Set(Some(cover_url));
        }
    }
    source
}

/// Deletes the stored cover files of `sources`, e.g. right before deleting the sources.
pub async fn delete_source_covers(sources: &[entity::manga_source::Model]) {
    for key in sources.iter().filter_map(|source| source.cover_storage_key.as_ref()) {
        if let Err(e) = crate::IMAGE_STORE.delete(key).await {
            warn!("Failed to delete cover {}: {}", key, e);
        }
    }
}

/// Create a manga from a freshly-scraped URL, with this as its (first, and therefore
/// primary) source - or, when its titles match a manga the catalog already has
/// (`title_match`), add it to that manga as a secondary source instead.
pub async fn create_manga(db: &DatabaseConnection, url: Url) -> Result<i32, Status> {
    info!("Creating manga [{}]", url);

    let manga: manga_parser::model::Manga =
        crate::util::scrape_log::record(db, "manga", &url, None, None, MANGA_PARSER.manga(&url)).await?;

    if let Some(manga_id) = util::title_match::find_title_match(db, &manga).await? {
        insert_scraped_source(db, manga_id, &manga, false).await?;
        return Ok(manga_id);
    }

    let saved_manga = entity::manga::ActiveModel { ..Default::default() }.insert(db).await.map_err(internal)?;
    if let Err(e) = insert_scraped_source(db, saved_manga.id, &manga, true).await {
        // Don't leave a manga without sources behind. The usual cause is the same url
        // being saved meanwhile (the search saver, another user); then that one is it.
        entity::manga::Entity::delete_by_id(saved_manga.id).exec(db).await.map_err(internal)?;
        let existing = entity::manga_source::Entity::find()
            .filter(entity::manga_source::Column::Url.eq(manga.url.to_string()))
            .one(db)
            .await
            .map_err(internal)?;
        return existing.map(|source| source.manga_id).ok_or(e);
    }

    Ok(saved_manga.id)
}

/// Stores `manga` as a source of `manga_id` with the details it was scraped with, inserts
/// its chapters and runs the matching heuristic.
async fn insert_scraped_source(
    db: &DatabaseConnection,
    manga_id: i32,
    manga: &manga_parser::model::Manga,
    is_primary: bool,
) -> Result<i32, Status> {
    let source = entity::manga_source::ActiveModel {
        manga_id: Set(manga_id),
        url: Set(manga.url.to_string()),
        hostname: Set(hostname_of(&manga.url)),
        is_primary: Set(is_primary),
        ..Default::default()
    };
    let saved_source = apply_scraped_details(source, None, manga)
        .await
        .insert(db)
        .await
        .map_err(internal)?;

    sync_chapters_for_source(db, manga_id, saved_source.id, &saved_source.url, false, &manga.chapters).await?;

    if manga.nsfw {
        set_manga_nsfw(db, manga_id, true).await?;
    }

    Ok(saved_source.id)
}

async fn set_manga_nsfw(db: &DatabaseConnection, manga_id: i32, nsfw: bool) -> Result<(), Status> {
    entity::manga::ActiveModel {
        id: Set(manga_id),
        nsfw: Set(nsfw),
        ..Default::default()
    }
    .update(db)
    .await
    .map_err(internal)?;
    Ok(())
}

/// For a user who turned off 18+ manga: leaves them out, except the ones they read.
/// Needs `select_manga`'s join on the user's reading row.
fn hide_nsfw_for(query: Select<entity::manga::Entity>, user: &entity::user::Model) -> Select<entity::manga::Entity> {
    if user.show_nsfw {
        query
    } else {
        query.filter(Expr::cust("(NOT manga.nsfw OR reading.manga_id IS NOT NULL)"))
    }
}

/// Add a new (by default, secondary) source to an existing manga: scrapes via the existing
/// call path, creates the manga_source row with the details it found, inserts chapters
/// against it, runs the matching heuristic.
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

    insert_scraped_source(db, manga_id, &manga, false).await
}

/// Refresh an existing manga_source in place: re-scrape its url, update its details and
/// sync its chapters.
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

    let active = entity::manga_source::ActiveModel {
        id: Set(source.id),
        ..Default::default()
    };
    apply_scraped_details(active, Some(&source), &manga)
        .await
        .update(db)
        .await
        .map_err(internal)?;

    sync_chapters_for_source(db, source.manga_id, manga_source_id, &source.url, force, &manga.chapters).await?;

    if manga.nsfw {
        set_manga_nsfw(db, source.manga_id, true).await?;
    }

    Ok(source.manga_id)
}

pub fn index_manga(logged_in: Option<&entity::user::Model>) -> Select<entity::manga::Entity> {
    select_manga(entity::manga::Entity::find(), logged_in)
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
        let logged_in = request.authorize()?;
        let req = request.get_ref();
        let manga_id = req.id;

        let manga = get_manga_by_id(db, Some(logged_in), manga_id).await?;
        if refresh_if_stale(db, manga.selected_source_id).await? {
            return Ok(Response::new(get_manga_by_id(db, Some(logged_in), manga_id).await?));
        }

        Ok(Response::new(manga))
    }

    /// Force update a manga - refreshes the source this user sees it through. `force` also
    /// pushes a chapter reset through even if it looks like a suspicious drop (see
    /// `sync_chapters_for_source`) -- for when someone has checked the source and confirmed
    /// the drop is real.
    async fn update(&self, request: Request<UpdateMangaRequest>) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?;
        let req = request.get_ref();
        let manga_id = req.id;

        let source_id = get_manga_by_id(db, Some(logged_in), manga_id).await?.selected_source_id;
        if source_id == 0 {
            return Err(Status::not_found("Manga has no source to refresh"));
        }

        info!("Updating manga with id '{}' (source {})", manga_id, source_id);
        refresh_manga_source(db, source_id, req.force).await?;

        Ok(Response::new(get_manga_by_id(db, Some(logged_in), manga_id).await?))
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
        let logged_in = request.authorize()?.clone();
        let req = request.get_ref();
        let per_page = req.per_page.unwrap_or(10).clamp(1, 50);
        let mut paginate = hide_nsfw_for(index_manga(Some(&logged_in)), &logged_in);

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
            paginate = paginate.order_by(
                Expr::col((entity::manga_source::Entity, entity::manga_source::Column::Title)),
                migration::Order::Asc,
            );
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
        let logged_in = request.authorize()?.clone();

        let id = request.get_ref().id;
        let titles: Vec<(String, Vec<String>)> = entity::manga_source::Entity::find()
            .filter(entity::manga_source::Column::MangaId.eq(id))
            .select_only()
            .column(entity::manga_source::Column::Title)
            .column(entity::manga_source::Column::AltTitles)
            .into_tuple()
            .all(db)
            .await
            .map_err(internal)?;
        if titles.is_empty() {
            return Err(Status::not_found("Manga not found"));
        }

        // Any title of any of this manga's sources against any title of any source of another.
        let mut titles: Vec<String> = titles
            .into_iter()
            .flat_map(|(title, alt_titles)| std::iter::once(title).chain(alt_titles))
            .filter(|title| !title.is_empty())
            .collect();
        titles.sort();
        titles.dedup();
        let title_matches = Expr::cust_with_values::<_, sea_orm::Value, _>(
            "EXISTS (SELECT 1 FROM manga_source s, unnest($1::text[]) t \
             WHERE s.manga_id = manga.id AND t % any(s.alt_titles || s.title))",
            [titles.into()],
        );

        let similar = hide_nsfw_for(index_manga(Some(&logged_in)), &logged_in)
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

    /// Remove a source from a manga, and its stored cover. If it's the primary source and
    /// another source still exists, that other source is auto-promoted to primary first.
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
        delete_source_covers(std::slice::from_ref(&source)).await;
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

    /// Change which source is primary for a manga: the one users see it through when they
    /// haven't picked a source themselves, and the one the auto-updater refreshes.
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

        Ok(Response::new(
            get_manga_by_id(db, Some(&logged_in), source.manga_id).await?,
        ))
    }

    /// Remember a source as the one this user sees its manga through (only for manga in
    /// their reading list, which is where the choice is stored) and refresh its details when
    /// they're stale, so switching to it shows what that site has.
    async fn select_source(&self, request: Request<SelectSourceRequest>) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?;
        let source_id = request.get_ref().manga_source_id;

        let source = entity::manga_source::Entity::find_by_id(source_id)
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("Manga source not found"))?;

        entity::reading::Entity::update_many()
            .col_expr(entity::reading::Column::PreferredSourceId, Expr::value(source.id))
            .filter(entity::reading::Column::UserId.eq(logged_in.id))
            .filter(entity::reading::Column::MangaId.eq(source.manga_id))
            .exec(db)
            .await
            .map_err(internal)?;

        if let Err(e) = refresh_if_stale(db, source.id).await {
            // Still switch to it: its chapters may be downloaded, or the site only briefly down.
            warn!("Failed to refresh selected source {}: {:#?}", source.id, e);
        }

        Ok(Response::new(get_manga_by_id(db, Some(logged_in), source.manga_id).await?))
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

    /// Mod/admin-only. Deletes a manga and its sources' stored covers; its sources, chapters
    /// and reading rows cascade.
    async fn delete(&self, request: Request<DeleteMangaRequest>) -> Result<Response<Empty>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?;
        if !logged_in.has_permission(UserPermissions::MOD) && !logged_in.has_permission(UserPermissions::ADMIN) {
            return Err(Status::permission_denied("Mod or admin permission required"));
        }
        let id = request.get_ref().id;

        let sources = entity::manga_source::Entity::find()
            .filter(entity::manga_source::Column::MangaId.eq(id))
            .all(db)
            .await
            .map_err(internal)?;
        let result = entity::manga::Entity::delete_by_id(id).exec(db).await.map_err(internal)?;
        delete_source_covers(&sources).await;
        if result.rows_affected == 0 {
            return Err(Status::not_found("Manga not found"));
        }

        info!("User {} deleted manga {}", logged_in.id, id);
        Ok(Response::new(Empty::default()))
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

    async fn for_you(&self, request: Request<PaginateQuery>) -> Result<Response<MangasReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?.clone();
        let req = request.get_ref();
        let per_page = req.per_page.unwrap_or(10).clamp(1, 50);
        let page = req.page.unwrap_or(0);

        let ranked = util::for_you::ranked_for_user(db, &logged_in).await?;
        let total = ranked.len() as u64;
        let ids: Vec<i32> = ranked
            .into_iter()
            .skip((page * per_page) as usize)
            .take(per_page as usize)
            .collect();

        let mut items = index_manga(Some(&logged_in))
            .filter(entity::manga::Column::Id.is_in(ids.clone()))
            .into_model::<data::manga::Full>()
            .all(db)
            .await
            .map_err(internal)?;
        items.sort_by_key(|item| ids.iter().position(|id| *id == item.id));

        Ok(Response::new(MangasReply {
            pagination: Some(PaginateReply {
                page,
                per_page,
                max_page: total.saturating_sub(1) / per_page,
                total,
            }),
            items: attach_sources(db, items).await?,
        }))
    }

    async fn dismiss(&self, request: Request<DismissMangaRequest>) -> Result<Response<Empty>, Status> {
        let db = request.db()?;
        let user_id = request.authorize()?.id;
        let manga_id = request.get_ref().id;

        entity::manga::Entity::find_by_id(manga_id)
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("Manga not found"))?;

        entity::manga_dismissal::Entity::insert(entity::manga_dismissal::ActiveModel {
            user_id: Set(user_id),
            manga_id: Set(manga_id),
            created_at: Set(Utc::now().naive_utc()),
        })
        .on_conflict(
            OnConflict::columns([
                entity::manga_dismissal::Column::UserId,
                entity::manga_dismissal::Column::MangaId,
            ])
            .update_column(entity::manga_dismissal::Column::CreatedAt)
            .to_owned(),
        )
        .exec(db)
        .await
        .map_err(internal)?;

        Ok(Response::new(Empty::default()))
    }

    async fn set_nsfw(&self, request: Request<SetNsfwRequest>) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?.clone();
        if !logged_in.has_permission(UserPermissions::MOD) && !logged_in.has_permission(UserPermissions::ADMIN) {
            return Err(Status::permission_denied("Mod or admin permission required"));
        }
        let req = request.get_ref();

        entity::manga::Entity::find_by_id(req.manga_id)
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("Manga not found"))?;
        set_manga_nsfw(db, req.manga_id, req.nsfw).await?;
        info!("User {} set manga {} nsfw to {}", logged_in.id, req.manga_id, req.nsfw);

        Ok(Response::new(get_manga_by_id(db, Some(&logged_in), req.manga_id).await?))
    }

    async fn undismiss(&self, request: Request<DismissMangaRequest>) -> Result<Response<Empty>, Status> {
        let db = request.db()?;
        let user_id = request.authorize()?.id;

        entity::manga_dismissal::Entity::delete_by_id((user_id, request.get_ref().id))
            .exec(db)
            .await
            .map_err(internal)?;

        Ok(Response::new(Empty::default()))
    }
}

/// Refreshes source `source_id` when its details are older than `MANGA_UPDATE_INTERVAL_MS`
/// or were never scraped since sources got their own. Returns whether it refreshed.
async fn refresh_if_stale(db: &DatabaseConnection, source_id: i32) -> Result<bool, Status> {
    let Some(details_updated_at) = entity::manga_source::Entity::find_by_id(source_id)
        .select_only()
        .column(entity::manga_source::Column::DetailsUpdatedAt)
        .into_tuple::<Option<NaiveDateTime>>()
        .one(db)
        .await
        .map_err(internal)?
    else {
        return Ok(false);
    };

    let interval_ms: i64 = std::env::var("MANGA_UPDATE_INTERVAL_MS")
        .unwrap_or("3600000".to_string())
        .parse()
        .unwrap_or(3600000);
    let stale_before = Utc::now().naive_utc() - chrono::Duration::milliseconds(interval_ms);
    if details_updated_at.is_some_and(|updated_at| updated_at > stale_before) {
        return Ok(false);
    }

    info!("Updating stale manga source {}", source_id);
    refresh_manga_source(db, source_id, false).await?;
    Ok(true)
}

crate::export_service!(MangaServer, MangaController, auth = UserPermissions::USER);

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

        let manga = entity::manga::ActiveModel { ..Default::default() }
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

    async fn make_manga(db: &sea_orm::DatabaseConnection, _title: &str) -> entity::manga::Model {
        entity::manga::ActiveModel { ..Default::default() }
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

    /// Simulates the exact scenario a force-rescrape is meant to handle: an existing manga
    /// with a lot of chapters and a user with reading progress mid-way through, then a
    /// forced re-sync where the source now reports far fewer chapters (a big, "suspicious"
    /// drop that only `force: true` allows through). Verifies the user's reading progress
    /// still resolves to a valid, correctly-ordinaled canonical chapter afterwards, and that
    /// chapters that survived the drop keep the *same* canonical_chapter_id they had before -
    /// not a fresh, different one.
    #[ignore]
    #[tokio::test]
    async fn force_rescrape_with_drop_preserves_reading_progress() {
        let db_url = std::env::var("DATABASE_URL").expect("set DATABASE_URL to a throwaway test DB");
        let db = Database::connect(db_url).await.unwrap();

        let manga = make_manga(&db, "Force Rescrape Manga").await;
        let source = make_source(&db, manga.id, "force-rescrape.test", true).await;
        let user = make_user(&db, "rescrapeuser").await;

        let total = 1000;
        let initial_chapters: Vec<Chapter> = (1..=total).map(|i| chapter("force-a", i as f32)).collect();
        sync_chapters_for_source(&db, manga.id, source.id, &source.url, false, &initial_chapters)
            .await
            .unwrap();

        // User is reading chapter 500.
        let chapter_500 = entity::chapter::Entity::find()
            .filter(entity::chapter::Column::MangaSourceId.eq(source.id))
            .filter(entity::chapter::Column::Number.eq(500.0))
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let canonical_id_before = chapter_500.canonical_chapter_id.unwrap();
        make_reading(&db, user.id, manga.id, canonical_id_before).await;

        // Force-rescrape: source now only reports chapters 1..=100 (a 90% drop - well past
        // the suspicious-drop threshold, only lets through because force: true).
        let shrunk_chapters: Vec<Chapter> = (1..=100).map(|i| chapter("force-a", i as f32)).collect();
        sync_chapters_for_source(&db, manga.id, source.id, &source.url, true, &shrunk_chapters)
            .await
            .unwrap();

        // The chapters that survived the drop (1..=100) must still exist, and chapter 50
        // must be linked to a canonical_chapter at ordinal 50 - the same canonical row, not
        // a newly-created duplicate.
        let remaining = entity::chapter::Entity::find()
            .filter(entity::chapter::Column::MangaSourceId.eq(source.id))
            .all(&db)
            .await
            .unwrap();
        assert_eq!(remaining.len(), 100, "only the 100 surviving chapters should remain");
        assert!(
            remaining.iter().all(|c| c.canonical_chapter_id.is_some()),
            "every surviving chapter must still be linked to a canonical chapter"
        );

        let canonical_count_50 = entity::canonical_chapter::Entity::find()
            .filter(entity::canonical_chapter::Column::MangaId.eq(manga.id))
            .filter(entity::canonical_chapter::Column::Ordinal.eq(sea_orm::prelude::Decimal::new(50000, 3)))
            .count(&db)
            .await
            .unwrap();
        assert_eq!(
            canonical_count_50, 1,
            "chapter 50 should re-link to its existing canonical row, not create a second one"
        );

        // The reading row set up before the rescrape must still resolve to a real,
        // correctly-ordinaled canonical_chapter - the rescrape must never have deleted or
        // orphaned canonical_chapter rows themselves (only `chapter` rows get reset).
        let reading = entity::reading::Entity::find_by_id((user.id, manga.id))
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            reading.last_canonical_chapter_id,
            Some(canonical_id_before),
            "user's reading progress must still point at the same canonical chapter as before the rescrape"
        );
        let still_valid = entity::canonical_chapter::Entity::find_by_id(canonical_id_before)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(still_valid.ordinal, sea_orm::prelude::Decimal::new(500000, 3));

        // Cleanup
        entity::manga::Entity::delete_by_id(manga.id).exec(&db).await.unwrap();
        entity::user::Entity::delete_by_id(user.id).exec(&db).await.unwrap();
    }
}
