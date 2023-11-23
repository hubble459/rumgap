use std::time::Duration;

use manga_parser::scraper::MangaScraper;
use migration::{Expr, IntoCondition, JoinType};
use sea_orm::{
    ColumnTrait, DeriveColumn, EntityTrait, EnumIter, QueryFilter, QuerySelect,
    RelationTrait,
};
use tokio::time::timeout;
use tonic::{Request, Response, Status};

use crate::proto::search_server::{Search, SearchServer};
use crate::proto::{SearchManga, SearchReply, SearchRequest};
use crate::MANGA_PARSER;
use crate::util::db::DatabaseRequest;
use crate::util::scrape_error_proto::StatusWrapper;

#[derive(Debug, Default)]
pub struct SearchController;

#[derive(Copy, Clone, Debug, EnumIter, DeriveColumn)]
enum QueryAs {
    Id,
    Url,
    Progress,
}

#[tonic::async_trait]
impl Search for SearchController {
    /// Edit reading progress
    async fn manga(
        &self,
        request: Request<SearchRequest>,
    ) -> Result<Response<SearchReply>, Status> {
        let db = request.db()?;
        let logged_in = request.extensions().get::<entity::user::Model>();
        let req = request.get_ref();

        let search_results = timeout(
            Duration::from_secs(5),
            MANGA_PARSER.search(&req.keyword, req.hostnames.as_slice()),
        )
        .await
        .map_err(|e| Status::deadline_exceeded(e.to_string()))?
        .map_err(StatusWrapper::from)?;

        let urls: Vec<String> = search_results
            .iter()
            .map(|item| item.url.to_string())
            .collect();

        let query = if let Some(logged_in) = logged_in {
            let user_id = logged_in.id;
            entity::manga::Entity::find().join(
                JoinType::LeftJoin,
                entity::reading::Relation::Manga
                    .def()
                    .rev()
                    .on_condition(move |_left, right| {
                        Expr::col((right, entity::reading::Column::UserId))
                            .eq(user_id)
                            .into_condition()
                    }),
            )
        } else {
            entity::manga::Entity::find()
        };

        let exists: Vec<(i32, String, Option<i32>)> = query
            .select_only()
            .columns([entity::manga::Column::Id, entity::manga::Column::Url])
            .column_as(entity::reading::Column::Progress, "progress")
            .filter(entity::manga::Column::Url.is_in(urls))
            .into_values::<_, QueryAs>()
            .all(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(SearchReply {
            items: search_results
                .into_iter()
                .map(|item| {
                    let existing = exists
                        .iter()
                        .find(|(_id, url, ..)| &item.url.to_string() == url);

                    SearchManga {
                        url: item.url.to_string(),
                        title: item.title,
                        cover: item.cover_url.map(|cover| cover.to_string()),
                        posted: item.posted.map(|date| date.timestamp_millis()),
                        is_reading: existing
                            .map_or(false, |(_id, _url, progress)| progress.is_some()),
                        manga_id: existing.map(|(id, ..)| *id),
                    }
                })
                .collect(),
        }))
    }
}

crate::export_service!(SearchServer, SearchController);
