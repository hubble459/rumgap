use std::num::TryFromIntError;

use migration::{Expr, ExprTrait, JoinType};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
    QueryTrait, RelationTrait, Select,
};
use tonic::{Request, Response, Status};

use crate::data;
use crate::proto::chapter_server::{Chapter, ChapterServer};
use crate::proto::{
    ChapterReply, ChapterRequest, ChaptersReply, FindEquivalentRequest, Id, ImagePage, ImagesReply, LinkChapterRequest,
    PaginateChapterQuery, PaginateReply, UnlinkChapterRequest,
};
use crate::util::auth::Authorize;
use crate::util::chapter_images::{ensure_chapter_image_rows, refresh_chapter_images};
use crate::util::db::DatabaseRequest;

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

/// Build the `{IMAGE_BASE_URL}/images/{chapter_id}/{page_index}` URLs (plus known
/// dimensions, if any) returned by `Chapter.Images`/`Chapter.RefreshImages`.
/// Deterministic and cheap -- never blocks on a download, that happens lazily inside
/// the image HTTP server on first real request for a given page.
fn image_pages(chapter_id: i32, rows: &[entity::chapter_image::Model]) -> Vec<ImagePage> {
    let base_url = std::env::var("IMAGE_BASE_URL").unwrap_or_else(|_| "http://localhost:8001".to_string());
    let base_url = base_url.trim_end_matches('/');

    rows.iter()
        .map(|row| ImagePage {
            url: format!("{base_url}/images/{chapter_id}/{}", row.page_index),
            width: row.width,
            height: row.height,
        })
        .collect()
}

fn internal<E: ToString>(e: E) -> Status {
    Status::internal(e.to_string())
}

#[derive(Debug, Default)]
pub struct ChapterController;

#[tonic::async_trait]
impl Chapter for ChapterController {
    /// Get chapter images. Never blocks on downloads -- only ensures
    /// `chapter_image` rows exist (scraping via manga_parser exactly once,
    /// ever, per chapter) and returns local image-server URLs.
    async fn images(&self, request: Request<Id>) -> Result<Response<ImagesReply>, Status> {
        let db = request.db()?;
        let req = request.get_ref();
        let chapter_id = req.id;

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
    async fn refresh_images(&self, request: Request<Id>) -> Result<Response<ImagesReply>, Status> {
        let db = request.db()?;
        let req = request.get_ref();
        let chapter_id = req.id;

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
        let logged_in = request.authorize().ok();
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
            .order_by(entity::chapter::Column::Id, migration::Order::Asc)
            .filter(entity::chapter::Column::MangaSourceId.eq(manga_source_id))
            .offset(offset)
            .column_as(Expr::cust("null"), "offset")
            .column_as(Expr::cust("null"), "page")
            .column_as(Expr::cust("null"), "fraction")
            .apply_if(logged_in, |query, logged_in| {
                let user_id = logged_in.id;
                query
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
            })
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
        let logged_in = request.authorize().ok();
        let req = request.get_ref();
        let manga_source_id = req.manga_source_id;
        let reversed = req.reversed.unwrap_or_default();
        let order = if reversed {
            migration::Order::Asc
        } else {
            migration::Order::Desc
        };
        let req = req.paginate_query.unwrap_or_default();
        let per_page = req.per_page.unwrap_or(10).clamp(1, 50);

        // Create paginate object
        let paginate = with_ordinal(entity::chapter::Entity::find())
            .filter(entity::chapter::Column::MangaSourceId.eq(manga_source_id))
            .order_by(entity::chapter::Column::Id, order)
            .column_as(Expr::cust("null"), "offset")
            .column_as(Expr::cust("null"), "page")
            .column_as(Expr::cust("null"), "fraction")
            .apply_if(logged_in, |query, logged_in| {
                let user_id = logged_in.id;
                query
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
            })
            .into_model::<data::chapter::Full>()
            .paginate(db, per_page);

        // Get max page and total items
        let amount = paginate.num_items_and_pages().await.map_err(internal)?;

        let max_page = if amount.number_of_pages == 0 {
            0
        } else {
            amount.number_of_pages - 1
        };

        // Don't clamp to max_page - infinite scroll relies on an out-of-range page coming
        // back empty, not repeating the last page forever.
        let page = std::cmp::Ord::max(req.page.unwrap_or(0), 0);

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
        request.authorize().ok();
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

crate::export_service!(ChapterServer, ChapterController);
