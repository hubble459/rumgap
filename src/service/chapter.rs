use manga_parser::parser::Parser;
use manga_parser::Url;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
};
use tonic::{Request, Response, Status};

use super::manga::MANGA_PARSER;
use crate::proto::chapter_server::{Chapter, ChapterServer};
use crate::proto::{
    ChapterReply, ChaptersReply, Id, ImagesReply, PaginateChapterQuery, PaginateReply,
};

#[derive(Debug, Default)]
pub struct MyChapter {}

#[tonic::async_trait]
impl Chapter for MyChapter {
    /// Get chapter images
    async fn get(&self, request: Request<Id>) -> Result<Response<ImagesReply>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let req = request.get_ref();
        let chapter_id = req.id;

        // Get chapter
        let chapter = entity::chapter::Entity::find_by_id(chapter_id)
            .one(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or(Status::not_found("Manga not found"))?;

        // Get images
        let images = MANGA_PARSER
            .images(&Url::parse(&chapter.url).unwrap())
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(ImagesReply {
            items: images.into_iter().map(|url| url.to_string()).collect(),
        }))
    }

    /// Get paginated chapters from a manga
    async fn index(
        &self,
        request: Request<PaginateChapterQuery>,
    ) -> Result<Response<ChaptersReply>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let req = request.get_ref();
        let manga_id = req.id;
        let req = req.paginate_query.clone().unwrap_or_default();
        let per_page = req.per_page.unwrap_or(10).clamp(1, 50);

        // Create paginate object
        let paginate = entity::chapter::Entity::find()
            .filter(entity::chapter::Column::MangaId.eq(manga_id))
            .order_by(entity::chapter::Column::Id, migration::Order::Asc)
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
                .map(|chapter| ChapterReply {
                    id: chapter.id,
                    manga_id: chapter.manga_id,
                    title: chapter.title,
                    url: chapter.url,
                    number: chapter.number,
                    posted: chapter.posted.map(|date| date.timestamp_millis()),
                    created_at: chapter.created_at.timestamp_millis(),
                    updated_at: chapter.updated_at.timestamp_millis(),
                })
                .collect(),
        }))
    }
}

crate::export_service!(ChapterServer, MyChapter);
