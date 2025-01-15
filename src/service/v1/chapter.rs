use std::num::TryFromIntError;

use manga_parser::scraper::MangaScraper;
use manga_parser::Url;
use migration::{Expr, IntoCondition, JoinType};
use sea_orm::{
    ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait, RelationTrait,
};
use tonic::{Request, Response, Status};

use crate::proto::chapter_server::{Chapter, ChapterServer};
use crate::proto::{ChapterReply, ChapterRequest, ChaptersReply, Id, ImagesReply, PaginateChapterQuery, PaginateReply};
use crate::util::auth::Authorize;
use crate::util::db::DatabaseRequest;
use crate::util::scrape_error_proto::StatusWrapper;
use crate::{data, MANGA_PARSER};

#[derive(Debug, Default)]
pub struct ChapterController;

#[tonic::async_trait]
impl Chapter for ChapterController {
    /// Get chapter images
    async fn images(&self, request: Request<Id>) -> Result<Response<ImagesReply>, Status> {
        let db = request.db()?;
        let req = request.get_ref();
        let chapter_id = req.id;

        // Get chapter
        let chapter = entity::chapter::Entity::find_by_id(chapter_id)
            .one(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or(Status::not_found("Chapter not found"))?;

        // Get images
        let images = MANGA_PARSER
            .chapter_images(&Url::parse(&chapter.url).unwrap())
            .await
            .map_err(StatusWrapper::from)?;

        debug!("{} images found in {}", images.len(), chapter.url);

        Ok(Response::new(ImagesReply {
            items: images.into_iter().map(|url| url.to_string()).collect(),
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
            .clamp(1, total_chapters.max(1) as i32)
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
                                    .into_condition()
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
                                    .into_condition()
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
