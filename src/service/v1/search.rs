use std::time::Duration;

use manga_parser::scraper::MangaScraper;
use migration::{Expr, ExprTrait, JoinType};
use sea_orm::{ColumnTrait, DeriveColumn, EntityTrait, EnumIter, QueryFilter, QuerySelect, RelationTrait};
use tokio::time::timeout;
use tonic::{Request, Response, Status};

use crate::interceptor::auth::UserPermissions;
use crate::proto::search_server::{Search, SearchServer};
use crate::proto::{SearchManga, SearchReply, SearchRequest};
use crate::service::v1::manga::find_suggested_manga_id;
use crate::util::auth::Authorize;
use crate::util::db::DatabaseRequest;
use crate::util::scrape_error_proto::StatusWrapper;
use crate::util::signed_url;
use crate::MANGA_PARSER;

#[derive(Debug, Default)]
pub struct SearchController;

#[derive(Copy, Clone, Debug, EnumIter, DeriveColumn)]
enum QueryAs {
    MangaId,
    Url,
    Progress,
}

#[tonic::async_trait]
impl Search for SearchController {
    /// Search for manga
    async fn manga(&self, request: Request<SearchRequest>) -> Result<Response<SearchReply>, Status> {
        let db = request.db()?;
        let user_id = request.authorize()?.id;
        let req = request.get_ref();

        let search_results = timeout(
            Duration::from_secs(10),
            MANGA_PARSER.search(&req.keyword, req.hostnames.as_slice()),
        )
        .await
        .map_err(|e| Status::deadline_exceeded(e.to_string()))?
        .map_err(StatusWrapper::from)?;

        let urls: Vec<String> = search_results.iter().map(|item| item.url.to_string()).collect();

        // manga_source is now the source of truth for urls; a manga's canonical `manga_id`
        // is already a plain column on manga_source, so no join through `manga` is needed
        // to get it - only to bring in this user's reading progress, which takes hopping
        // manga_source -> manga -> reading since `reading` only has a direct FK to `manga`,
        // not `manga_source`.
        let exists: Vec<(i32, String, Option<i32>)> = entity::manga_source::Entity::find()
            .join(JoinType::LeftJoin, entity::manga_source::Relation::Manga.def())
            .join(
                JoinType::LeftJoin,
                entity::reading::Relation::Manga
                    .def()
                    .rev()
                    .on_condition(move |_left, right| {
                        Expr::col((right, entity::reading::Column::UserId)).eq(user_id).into()
                    }),
            )
            .select_only()
            .column(entity::manga_source::Column::MangaId)
            .column(entity::manga_source::Column::Url)
            .column_as(entity::reading::Column::Progress, "progress")
            .filter(entity::manga_source::Column::Url.is_in(urls))
            .into_values::<_, QueryAs>()
            .all(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let mut items = Vec::with_capacity(search_results.len());
        for item in search_results {
            let existing = exists.iter().find(|(_id, url, ..)| &item.url.to_string() == url);
            let manga_id = existing.map(|(id, ..)| *id);

            let suggested_manga_id = if manga_id.is_none() {
                find_suggested_manga_id(db, &item.title).await?
            } else {
                None
            };

            items.push(SearchManga {
                url: item.url.to_string(),
                title: item.title,
                cover: item
                    .cover_url
                    .map(|cover| signed_url::proxy_url(cover.as_ref(), item.url.as_ref())),
                posted: item.posted.map(|date| date.timestamp_millis()),
                is_reading: existing.is_some_and(|(_id, _url, progress)| progress.is_some()),
                manga_id,
                suggested_manga_id,
            });
        }

        Ok(Response::new(SearchReply { items }))
    }
}

crate::export_service!(SearchServer, SearchController, auth = UserPermissions::USER);
