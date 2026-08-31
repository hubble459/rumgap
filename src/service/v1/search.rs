use std::time::Duration;

use manga_parser::scraper::MangaScraper;
use migration::{Expr, ExprTrait, JoinType};
use sea_orm::{ColumnTrait, DeriveColumn, EntityTrait, EnumIter, QueryFilter, QuerySelect, RelationTrait};
use tokio::time::timeout;
use tonic::{Request, Response, Status};

use crate::proto::search_server::{Search, SearchServer};
use crate::proto::{SearchManga, SearchReply, SearchRequest};
use crate::service::v1::manga::find_suggested_manga_id;
use crate::util::auth::Authorize;
use crate::util::db::DatabaseRequest;
use crate::util::scrape_error_proto::StatusWrapper;
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
        let logged_in = request.authorize().ok();
        let req = request.get_ref();

        let search_results = timeout(
            Duration::from_secs(5),
            MANGA_PARSER.search(&req.keyword, req.hostnames.as_slice()),
        )
        .await
        .map_err(|e| Status::deadline_exceeded(e.to_string()))?
        .map_err(StatusWrapper::from)?;

        let urls: Vec<String> = search_results.iter().map(|item| item.url.to_string()).collect();

        // manga_source is now the source of truth for urls; a manga's canonical `manga_id`
        // is already a plain column on manga_source, so no join through `manga` is needed
        // to get it - only (when logged in) to bring in this user's reading progress, which
        // takes hopping manga_source -> manga -> reading since `reading` only has a direct
        // FK to `manga`, not `manga_source`.
        let query = if let Some(logged_in) = logged_in {
            let user_id = logged_in.id;
            entity::manga_source::Entity::find()
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
        } else {
            entity::manga_source::Entity::find()
        };

        let exists: Vec<(i32, String, Option<i32>)> = query
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
                    .map(|cover| proxy_cover_url(cover.as_ref(), item.url.as_ref())),
                posted: item.posted.map(|date| date.timestamp_millis()),
                is_reading: existing.is_some_and(|(_id, _url, progress)| progress.is_some()),
                manga_id,
                suggested_manga_id,
            });
        }

        Ok(Response::new(SearchReply { items }))
    }
}

/// Rewrite a raw scraped cover URL into rumgap's own transient `/proxy`
/// cache (Phase A2) -- the client never sees or needs the raw source URL,
/// and the server does the Referer-spoofing itself rather than the client
/// (which matters beyond tidiness: a browser build can't set `Referer` on
/// `fetch`/XHR at all, it's a forbidden header).
fn proxy_cover_url(cover_url: &str, referer_url: &str) -> String {
    let base_url = std::env::var("IMAGE_BASE_URL").unwrap_or_else(|_| "http://localhost:8001".to_string());
    let mut url = manga_parser::Url::parse(&format!("{}/proxy", base_url.trim_end_matches('/')))
        .expect("IMAGE_BASE_URL should be a valid base URL");
    url.query_pairs_mut()
        .append_pair("url", cover_url)
        .append_pair("referer", referer_url);
    url.to_string()
}

crate::export_service!(SearchServer, SearchController);
