use std::num::TryFromIntError;

use migration::{Expr, ExprTrait, JoinType};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, FromQueryResult,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, RelationTrait, Select, Statement,
};
use tonic::{Request, Response, Status};

use crate::data;
use crate::interceptor::auth::UserPermissions;
use crate::proto::chapter_server::{Chapter, ChapterServer};
use crate::proto::{
    ChapterImagesRequest, ChapterReply, ChapterRequest, ChaptersReply, FindEquivalentRequest, ImagePage, ImagesReply,
    LinkChapterRequest, PaginateChapterQuery, PaginateReply, RefreshImagesRequest, UnlinkChapterRequest,
};
use crate::util::auth::Authorize;
use crate::util::chapter_images::{ensure_chapter_image_rows, refresh_chapter_images};
use crate::util::db::DatabaseRequest;
use crate::util::signed_url;

/// LEFT JOIN this chapter's linked `canonical_chapter` (if any - NULL for a manually
/// `UnlinkChapter`'d chapter) and select its `ordinal`, so `ChapterReply.ordinal` can be
/// compared against `MangaReply.progress_ordinal` for "is this chapter read" - the two
/// numbers actually share a scale, unlike `chapter.index` (purely per-source position) vs
/// `reading_progress` (canonical rank), which don't once bonus/unlinked chapters exist.
fn with_ordinal(query: Select<entity::chapter::Entity>) -> Select<entity::chapter::Entity> {
    query
        .join(JoinType::LeftJoin, entity::chapter::Relation::CanonicalChapter.def())
        .column_as(entity::canonical_chapter::Column::Ordinal, "ordinal")
}

/// Build the signed `{IMAGE_BASE_URL}/images/{chapter_id}/{page_index}` URLs (plus known
/// dimensions, if any) returned by `Chapter.Images`/`Chapter.RefreshImages`.
/// Deterministic and cheap -- never blocks on a download, that happens lazily inside
/// the image HTTP server on first real request for a given page.
fn image_pages(chapter_id: i32, rows: &[entity::chapter_image::Model]) -> Vec<ImagePage> {
    rows.iter()
        .map(|row| ImagePage {
            url: signed_url::path_url(&format!("/images/{chapter_id}/{}", row.page_index)),
            width: row.width,
            height: row.height,
        })
        .collect()
}

fn internal<E: ToString>(e: E) -> Status {
    Status::internal(e.to_string())
}

/// `Chapter.Index` with `canonical`: a page of the manga's canonical chapters, each with
/// this source's chapter or a placeholder.
async fn canonical_index(
    db: &DatabaseConnection,
    user_id: i32,
    manga_source_id: i32,
    reversed: bool,
    page: u64,
    per_page: u64,
) -> Result<ChaptersReply, Status> {
    let backend = db.get_database_backend();
    let total: i64 = db
        .query_one_raw(Statement::from_sql_and_values(
            backend,
            "SELECT count(*) AS total FROM canonical_chapter \
             WHERE manga_id = (SELECT manga_id FROM manga_source WHERE id = $1)",
            [manga_source_id.into()],
        ))
        .await
        .map_err(internal)?
        .map(|row| row.try_get("", "total"))
        .transpose()
        .map_err(internal)?
        .unwrap_or(0);
    let total = total as u64;
    let max_page = total.div_ceil(per_page).saturating_sub(1);
    let page = std::cmp::Ord::min(page, max_page + 1);
    let order = if reversed { "ASC" } else { "DESC" };

    let rows = data::chapter::Canonical::find_by_statement(Statement::from_sql_and_values(
        backend,
        format!(
            r#"SELECT cc.id AS canonical_chapter_id, cc.ordinal,
                   c.id, c.url, c.title, c.number, c.posted, c.created_at, c.updated_at,
                   CASE WHEN c.id IS NULL THEN NULL ELSE (
                       SELECT count(*) FROM chapter s
                       WHERE s.manga_source_id = $1 AND (s.number, s.id) <= (c.number, c.id)
                   ) END AS source_index,
                   co."offset", co.page, co.fraction,
                   CASE WHEN c.id IS NULL THEN ARRAY(
                       SELECT DISTINCT o.manga_source_id FROM chapter o
                       WHERE o.canonical_chapter_id = cc.id ORDER BY o.manga_source_id
                   ) ELSE ARRAY[]::integer[] END AS available_source_ids
               FROM canonical_chapter cc
               LEFT JOIN LATERAL (
                   SELECT * FROM chapter
                   WHERE chapter.canonical_chapter_id = cc.id AND chapter.manga_source_id = $1
                   ORDER BY chapter.id LIMIT 1
               ) c ON true
               LEFT JOIN chapter_offset co ON co.chapter_id = c.id AND co.user_id = $2
               WHERE cc.manga_id = (SELECT manga_id FROM manga_source WHERE id = $1)
               ORDER BY cc.ordinal {order}, cc.id {order}
               LIMIT $3 OFFSET $4"#
        ),
        [
            manga_source_id.into(),
            user_id.into(),
            (per_page as i64).into(),
            ((page * per_page) as i64).into(),
        ],
    ))
    .all(db)
    .await
    .map_err(internal)?;

    Ok(ChaptersReply {
        pagination: Some(PaginateReply {
            page,
            per_page,
            max_page,
            total,
        }),
        items: rows
            .into_iter()
            .map(|row| row.into_chapter_reply(manga_source_id))
            .collect(),
    })
}

#[derive(Debug, Default)]
pub struct ChapterController;

#[tonic::async_trait]
impl Chapter for ChapterController {
    /// Get chapter images. Never blocks on downloads -- only ensures
    /// `chapter_image` rows exist (scraping via manga_parser exactly once,
    /// ever, per chapter) and returns local image-server URLs.
    async fn images(&self, request: Request<ChapterImagesRequest>) -> Result<Response<ImagesReply>, Status> {
        let db = request.db()?;
        let req = request.get_ref();
        let chapter_id = req.chapter_id;

        let chapter = entity::chapter::Entity::find_by_id(chapter_id)
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("Chapter not found"))?;

        let rows = ensure_chapter_image_rows(db, &chapter).await?;

        debug!("{} image row(s) ensured for chapter {}", rows.len(), chapter.url);

        Ok(Response::new(ImagesReply {
            items: image_pages(chapter_id, &rows),
        }))
    }

    /// Manual-only force-refresh: deletes this chapter's existing
    /// `chapter_image` rows (and their stored bytes) and re-scrapes/re-caches
    /// from scratch. For the handful of sites that briefly serve a
    /// placeholder/troll image right after a chapter is scraped.
    async fn refresh_images(&self, request: Request<RefreshImagesRequest>) -> Result<Response<ImagesReply>, Status> {
        let db = request.db()?;
        let req = request.get_ref();
        let chapter_id = req.chapter_id;

        let chapter = entity::chapter::Entity::find_by_id(chapter_id)
            .one(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or(Status::not_found("Chapter not found"))?;

        let rows = refresh_chapter_images(db, &chapter).await?;

        info!("Refreshed {} image row(s) for chapter {}", rows.len(), chapter.url);

        Ok(Response::new(ImagesReply {
            items: image_pages(chapter_id, &rows),
        }))
    }

    /// Get chapter
    async fn get(&self, request: Request<ChapterRequest>) -> Result<Response<ChapterReply>, Status> {
        let db = request.db()?;
        let user_id = request.authorize()?.id;
        let req = request.get_ref();
        let manga_source_id = req.manga_source_id;

        let total_chapters = entity::chapter::Entity::find()
            .filter(entity::chapter::Column::MangaSourceId.eq(manga_source_id))
            .count(db)
            .await
            .map_err(internal)?;

        let offset: u64 = req
            .index
            .clamp(1, std::cmp::Ord::max(total_chapters, 1) as i32)
            .try_into()
            .map_err(|e: TryFromIntError| Status::invalid_argument(e.to_string()))?;

        let offset = offset - 1;

        // Get chapter
        let chapter = with_ordinal(entity::chapter::Entity::find())
            .order_by(entity::chapter::Column::Number, migration::Order::Asc)
            .order_by(entity::chapter::Column::Id, migration::Order::Asc)
            .filter(entity::chapter::Column::MangaSourceId.eq(manga_source_id))
            .offset(offset)
            .column_as(Expr::cust("null"), "offset")
            .column_as(Expr::cust("null"), "page")
            .column_as(Expr::cust("null"), "fraction")
            .join(
                JoinType::LeftJoin,
                entity::chapter_offset::Relation::Chapter
                    .def()
                    .rev()
                    .on_condition(move |_left, right| {
                        Expr::col((right, entity::reading::Column::UserId)).eq(user_id).into()
                    }),
            )
            .column_as(entity::chapter_offset::Column::Offset, "offset")
            .column_as(entity::chapter_offset::Column::Page, "page")
            .column_as(entity::chapter_offset::Column::Fraction, "fraction")
            .into_model::<data::chapter::Full>()
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("Chapter not found"))?;

        Ok(Response::new(chapter.into_chapter_reply(offset as i64)))
    }

    /// Get paginated chapters from a manga source
    async fn index(&self, request: Request<PaginateChapterQuery>) -> Result<Response<ChaptersReply>, Status> {
        let db = request.db()?;
        let user_id = request.authorize()?.id;
        let req = request.get_ref();
        let manga_source_id = req.manga_source_id;
        let reversed = req.reversed.unwrap_or_default();
        let order = if reversed {
            migration::Order::Asc
        } else {
            migration::Order::Desc
        };
        let canonical = req.canonical.unwrap_or_default();
        let req = req.paginate_query.unwrap_or_default();
        let per_page = req.per_page.unwrap_or(10).clamp(1, 50);

        if canonical {
            return canonical_index(db, user_id, manga_source_id, reversed, req.page.unwrap_or(0), per_page)
                .await
                .map(Response::new);
        }

        // Create paginate object
        let paginate = with_ordinal(entity::chapter::Entity::find())
            .filter(entity::chapter::Column::MangaSourceId.eq(manga_source_id))
            .order_by(entity::chapter::Column::Number, order.clone())
            .order_by(entity::chapter::Column::Id, order)
            .column_as(Expr::cust("null"), "offset")
            .column_as(Expr::cust("null"), "page")
            .column_as(Expr::cust("null"), "fraction")
            .join(
                JoinType::LeftJoin,
                entity::chapter_offset::Relation::Chapter
                    .def()
                    .rev()
                    .on_condition(move |_left, right| {
                        Expr::col((right, entity::reading::Column::UserId)).eq(user_id).into()
                    }),
            )
            .column_as(entity::chapter_offset::Column::Offset, "offset")
            .column_as(entity::chapter_offset::Column::Page, "page")
            .column_as(entity::chapter_offset::Column::Fraction, "fraction")
            .into_model::<data::chapter::Full>()
            .paginate(db, per_page);

        // Get max page and total items
        let amount = paginate.num_items_and_pages().await.map_err(internal)?;

        let max_page = if amount.number_of_pages == 0 {
            0
        } else {
            amount.number_of_pages - 1
        };

        // Capped at max_page + 1 (not clamped down, so out-of-range still returns empty for infinite scroll) to stop a garbage huge page number from overflowing the SQL offset.
        let page = std::cmp::Ord::min(req.page.unwrap_or(0), max_page + 1);

        // Get items from page
        let items = paginate.fetch_page(page).await.map_err(internal)?;

        Ok(Response::new(ChaptersReply {
            pagination: Some(PaginateReply {
                page,
                per_page,
                max_page,
                total: amount.number_of_items,
            }),
            items: items
                .into_iter()
                .enumerate()
                .map(|(index, chapter)| {
                    chapter.into_chapter_reply(if reversed {
                        page as i64 * per_page as i64 + index as i64 + 1
                    } else {
                        amount.number_of_items as i64 - (page as i64 * per_page as i64) - index as i64
                    })
                })
                .collect(),
        }))
    }

    /// Manual override: link a chapter to a specific canonical_chapter (must belong to the
    /// same manga). Used when the automatic matching heuristic (Phase 1b) got it wrong.
    async fn link_chapter(&self, request: Request<LinkChapterRequest>) -> Result<Response<ChapterReply>, Status> {
        let db = request.db()?;
        request.authorize()?;
        let req = request.get_ref();

        let chapter = entity::chapter::Entity::find_by_id(req.chapter_id)
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("Chapter not found"))?;

        let source = entity::manga_source::Entity::find_by_id(chapter.manga_source_id)
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("Manga source not found"))?;

        let canonical = entity::canonical_chapter::Entity::find_by_id(req.canonical_chapter_id)
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("Canonical chapter not found"))?;

        if canonical.manga_id != source.manga_id {
            return Err(Status::invalid_argument(
                "canonical_chapter_id does not belong to the same manga as this chapter",
            ));
        }

        entity::chapter::ActiveModel {
            id: Set(chapter.id),
            canonical_chapter_id: Set(Some(canonical.id)),
            ..Default::default()
        }
        .update(db)
        .await
        .map_err(internal)?;

        get_chapter_reply(db, req.chapter_id).await
    }

    /// Manual override: detach a chapter from its canonical_chapter. This is the *only*
    /// case where `canonical_chapter_id` is meant to be NULL - every other chapter always
    /// ends up attached to some canonical_chapter (even a solo one).
    async fn unlink_chapter(&self, request: Request<UnlinkChapterRequest>) -> Result<Response<ChapterReply>, Status> {
        let db = request.db()?;
        request.authorize()?;
        let req = request.get_ref();

        entity::chapter::Entity::find_by_id(req.chapter_id)
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("Chapter not found"))?;

        entity::chapter::ActiveModel {
            id: Set(req.chapter_id),
            canonical_chapter_id: Set(None),
            ..Default::default()
        }
        .update(db)
        .await
        .map_err(internal)?;

        get_chapter_reply(db, req.chapter_id).await
    }

    /// Given a canonical chapter and a preferred manga_source, find that source's
    /// equivalent chapter row (if it has one) - the RPC behind "my preferred source
    /// doesn't have this one, what's the equivalent elsewhere?"
    async fn find_equivalent(&self, request: Request<FindEquivalentRequest>) -> Result<Response<ChapterReply>, Status> {
        let db = request.db()?;
        let req = request.get_ref();

        let chapter = with_ordinal(entity::chapter::Entity::find())
            .filter(entity::chapter::Column::CanonicalChapterId.eq(req.canonical_chapter_id))
            .filter(entity::chapter::Column::MangaSourceId.eq(req.manga_source_id))
            .column_as(Expr::cust("null"), "offset")
            .column_as(Expr::cust("null"), "page")
            .column_as(Expr::cust("null"), "fraction")
            .into_model::<data::chapter::Full>()
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("No equivalent chapter found on this source"))?;

        Ok(Response::new(chapter.into_chapter_reply(0)))
    }
}

async fn get_chapter_reply(
    db: &sea_orm::DatabaseConnection,
    chapter_id: i32,
) -> Result<Response<ChapterReply>, Status> {
    let chapter = entity::chapter::Entity::find_by_id(chapter_id)
        .column_as(Expr::cust("null"), "offset")
        .column_as(Expr::cust("null"), "page")
        .column_as(Expr::cust("null"), "fraction")
        .into_model::<data::chapter::Full>()
        .one(db)
        .await
        .map_err(internal)?
        .ok_or(Status::not_found("Chapter not found"))?;

    Ok(Response::new(chapter.into_chapter_reply(0)))
}

crate::export_service!(ChapterServer, ChapterController, auth = UserPermissions::USER);
