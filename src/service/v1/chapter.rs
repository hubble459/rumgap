use std::num::TryFromIntError;

use migration::{Expr, ExprTrait, JoinType};
use sea_orm::{
    ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait, RelationTrait,
};
use tonic::{Request, Response, Status};

use crate::data;
use crate::proto::chapter_server::{Chapter, ChapterServer};
use crate::proto::{
    BackfillStatusReply, ChapterReply, ChapterRequest, ChaptersReply, Empty, Id, ImagesReply, PaginateChapterQuery,
    PaginateReply,
};
use crate::util::auth::Authorize;
use crate::util::backfill;
use crate::util::chapter_images::{ensure_chapter_image_rows, refresh_chapter_images};
use crate::util::db::DatabaseRequest;

/// Build the `{IMAGE_BASE_URL}/images/{chapter_id}/{page_index}` URLs
/// returned by `Chapter.Images`/`Chapter.RefreshImages`. Deterministic and
/// cheap -- never blocks on a download, that happens lazily inside the
/// image HTTP server on first real request for a given page.
fn image_urls(chapter_id: i32, rows: &[entity::chapter_image::Model]) -> Vec<String> {
    let base_url = std::env::var("IMAGE_BASE_URL").unwrap_or_else(|_| "http://localhost:8001".to_string());
    let base_url = base_url.trim_end_matches('/');

    rows.iter()
        .map(|row| format!("{base_url}/images/{chapter_id}/{}", row.page_index))
        .collect()
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
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or(Status::not_found("Chapter not found"))?;

        let rows = ensure_chapter_image_rows(db, &chapter).await?;

        debug!("{} image row(s) ensured for chapter {}", rows.len(), chapter.url);

        Ok(Response::new(ImagesReply {
            items: image_urls(chapter_id, &rows),
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
            items: image_urls(chapter_id, &rows),
        }))
    }

    /// Id = manga_source_id (see the deviation note on this RPC in
    /// `proto/rumgap/v1/v1.proto`). Kicks off (or resumes) a throttled
    /// background walk downloading every not-yet-cached page of every
    /// chapter of that source.
    async fn backfill_images(&self, request: Request<Id>) -> Result<Response<Empty>, Status> {
        let db = request.db()?.clone();
        let manga_source_id = request.get_ref().id;

        backfill::start_backfill(db, manga_source_id);

        Ok(Response::new(Empty::default()))
    }

    /// Id = manga_source_id. Cheap `GROUP BY status` progress count for the
    /// client to poll on its own schedule.
    async fn get_backfill_status(&self, request: Request<Id>) -> Result<Response<BackfillStatusReply>, Status> {
        let db = request.db()?;
        let manga_source_id = request.get_ref().id;

        let (images_downloaded, images_total) = backfill::backfill_status(db, manga_source_id).await?;

        Ok(Response::new(BackfillStatusReply {
            images_downloaded,
            images_total,
        }))
    }

    /// Get chapter
    async fn get(&self, request: Request<ChapterRequest>) -> Result<Response<ChapterReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize().ok();
        let req = request.get_ref();
        let manga_id = req.manga_id;

        let total_chapters = entity::chapter::Entity::find()
            .filter(entity::chapter::Column::MangaId.eq(manga_id))
            .count(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let offset: u64 = req
            .index
            .clamp(1, std::cmp::Ord::max(total_chapters, 1) as i32)
            .try_into()
            .map_err(|e: TryFromIntError| Status::invalid_argument(e.to_string()))?;

        let offset = offset - 1;

        // Get chapter
        let chapter = entity::chapter::Entity::find()
            .order_by(entity::chapter::Column::Id, migration::Order::Asc)
            .filter(entity::chapter::Column::MangaId.eq(manga_id))
            .offset(offset)
            .column_as(Expr::cust("null"), "offset")
            .column_as(Expr::cust("null"), "page")
            .apply_if(logged_in, |query, logged_in| {
                let user_id = logged_in.id;
                query
                    .join(
                        JoinType::LeftJoin,
                        entity::chapter_offset::Relation::Chapter
                            .def()
                            .rev()
                            .on_condition(move |_left, right| {
                                Expr::col((right, entity::reading::Column::UserId))
                                    .eq(user_id)
                                    .into()
                            }),
                    )
                    .column_as(entity::chapter_offset::Column::Offset, "offset")
                    .column_as(entity::chapter_offset::Column::Page, "page")
            })
            .into_model::<data::chapter::Full>()
            .one(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or(Status::not_found("Chapter not found"))?;

        Ok(Response::new(chapter.into_chapter_reply(offset as i64)))
    }

    /// Get paginated chapters from a manga
    async fn index(&self, request: Request<PaginateChapterQuery>) -> Result<Response<ChaptersReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize().ok();
        let req = request.get_ref();
        let manga_id = req.id;
        let reversed = req.reversed.unwrap_or_default();
        let order = if reversed {
            migration::Order::Asc
        } else {
            migration::Order::Desc
        };
        let req = req.paginate_query.clone().unwrap_or_default();
        let per_page = req.per_page.unwrap_or(10).clamp(1, 50);

        // Create paginate object
        let paginate = entity::chapter::Entity::find()
            .filter(entity::chapter::Column::MangaId.eq(manga_id))
            .order_by(entity::chapter::Column::Id, order)
            .column_as(Expr::cust("null"), "offset")
            .column_as(Expr::cust("null"), "page")
            .apply_if(logged_in, |query, logged_in| {
                let user_id = logged_in.id;
                query
                    .join(
                        JoinType::LeftJoin,
                        entity::chapter_offset::Relation::Chapter
                            .def()
                            .rev()
                            .on_condition(move |_left, right| {
                                Expr::col((right, entity::reading::Column::UserId))
                                    .eq(user_id)
                                    .into()
                            }),
                    )
                    .column_as(entity::chapter_offset::Column::Offset, "offset")
                    .column_as(entity::chapter_offset::Column::Page, "page")
            })
            .into_model::<data::chapter::Full>()
            .paginate(db, per_page);

        // Get max page and total items
        let amount = paginate
            .num_items_and_pages()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let max_page = if amount.number_of_pages == 0 {
            0
        } else {
            amount.number_of_pages - 1
        };

        let page = req.page.unwrap_or(0).clamp(0, max_page);

        // Get items from page
        let items = paginate
            .fetch_page(page)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

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
}

crate::export_service!(ChapterServer, ChapterController);
