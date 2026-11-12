use std::collections::HashSet;
use std::time::Duration;

use log::warn;
use manga_parser::error::ScrapeError;
use manga_parser::model::SearchManga as ScrapedSearchManga;
use manga_parser::scraper::MangaScraper;
use manga_parser::Url;
use migration::{Expr, ExprTrait, JoinType};
use sea_orm::{ColumnTrait, DeriveColumn, EntityTrait, EnumIter, QueryFilter, QuerySelect, RelationTrait};
use tokio::time::timeout;
use tonic::{Request, Response, Status};

use crate::interceptor::auth::UserPermissions;
use crate::proto::search_server::{Search, SearchServer};
use crate::proto::{SearchManga, SearchReply, SearchRequest};
use crate::util;
use crate::util::auth::Authorize;
use crate::util::db::DatabaseRequest;
use crate::util::scrape_error_proto::StatusWrapper;
use crate::util::signed_url;
use crate::MANGA_PARSER;

/// How long one hostname gets to answer before its results are left out.
const HOSTNAME_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Default)]
pub struct SearchController;

#[derive(Copy, Clone, Debug, EnumIter, DeriveColumn)]
enum QueryAs {
    MangaId,
    Url,
    Progress,
    Nsfw,
}

/// Searches each hostname on its own task with its own timeout, so a slow, hanging or
/// panicking site only loses its own results. Errors only when no hostname gave results.
async fn search_hostnames(keyword: &str, hostnames: Vec<String>) -> Result<Vec<ScrapedSearchManga>, Status> {
    let tasks: Vec<_> = hostnames
        .into_iter()
        .map(|hostname| {
            let keyword = keyword.to_string();
            let host = hostname.clone();
            let task =
                tokio::spawn(async move { timeout(HOSTNAME_TIMEOUT, MANGA_PARSER.search(&keyword, &[host])).await });
            (hostname, task)
        })
        .collect();

    let mut results = vec![];
    let mut last_error: Option<Status> = None;
    for (hostname, task) in tasks {
        match task.await {
            Ok(Ok(Ok(mut found))) => results.append(&mut found),
            Ok(Ok(Err(ScrapeError::SearchNotSupported(_)))) => {}
            Ok(Ok(Err(e))) => {
                warn!("[SEARCH] {hostname} failed: {e:?}");
                last_error = Some(StatusWrapper::from(e).into());
            }
            Ok(Err(_)) => {
                warn!("[SEARCH] {hostname} timed out");
                last_error = Some(Status::deadline_exceeded(format!("{hostname} timed out")));
            }
            Err(e) => {
                warn!("[SEARCH] {hostname} panicked: {e}");
                last_error = Some(Status::internal(format!("{hostname} failed")));
            }
        }
    }

    // Some scrapers (mangadex) answer for several hostnames with the same results.
    let mut seen = HashSet::new();
    results.retain(|item| seen.insert(item.url.to_string()));

    match last_error {
        Some(e) if results.is_empty() => Err(e),
        _ => Ok(results),
    }
}

#[tonic::async_trait]
impl Search for SearchController {
    /// Search for manga
    async fn manga(&self, request: Request<SearchRequest>) -> Result<Response<SearchReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?.clone();
        let user_id = logged_in.id;
        let req = request.get_ref();

        let hostnames = if req.hostnames.is_empty() {
            MANGA_PARSER.searchable_hostnames()
        } else {
            req.hostnames.clone()
        };
        let search_results = search_hostnames(&req.keyword, hostnames).await?;

        let urls: Vec<String> = search_results.iter().map(|item| item.url.to_string()).collect();

        // manga_source is now the source of truth for urls; a manga's canonical `manga_id`
        // is already a plain column on manga_source, so no join through `manga` is needed
        // to get it - only to bring in this user's reading progress, which takes hopping
        // manga_source -> manga -> reading since `reading` only has a direct FK to `manga`,
        // not `manga_source`.
        let exists: Vec<(i32, String, Option<i32>, bool)> = entity::manga_source::Entity::find()
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
            .column(entity::manga::Column::Nsfw)
            .filter(entity::manga_source::Column::Url.is_in(urls))
            .into_values::<_, QueryAs>()
            .all(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let unknown_titles = search_results
            .iter()
            .filter(|item| !exists.iter().any(|(_id, url, ..)| &item.url.to_string() == url))
            .map(|item| item.title.clone())
            .collect();
        let suggestions = util::title_match::find_title_matches(db, unknown_titles).await?;

        let items: Vec<SearchManga> = search_results
            .into_iter()
            .filter(|item| {
                logged_in.show_nsfw
                    || !(item.url.host_str().is_some_and(util::nsfw::is_nsfw_hostname)
                        || exists.iter().any(|(_id, url, progress, nsfw)| {
                            *nsfw && progress.is_none() && &item.url.to_string() == url
                        }))
            })
            .map(|item| {
                let existing = exists.iter().find(|(_id, url, ..)| &item.url.to_string() == url);
                let manga_id = existing.map(|(id, ..)| *id);
                let suggested_manga_id = if manga_id.is_none() {
                    suggestions.get(&item.title).copied()
                } else {
                    None
                };

                SearchManga {
                    url: item.url.to_string(),
                    cover: item
                        .cover_url
                        .map(|cover| signed_url::proxy_url(cover.as_ref(), item.url.as_ref())),
                    posted: item.posted.map(|date| date.timestamp_millis()),
                    is_reading: existing.is_some_and(|(_id, _url, progress, _nsfw)| progress.is_some()),
                    title: item.title,
                    manga_id,
                    suggested_manga_id,
                }
            })
            .collect();

        crate::util::search_save::enqueue(
            db,
            items
                .iter()
                .filter(|item| item.manga_id.is_none())
                .filter_map(|item| Url::parse(&item.url).ok()),
        );

        Ok(Response::new(SearchReply { items }))
    }
}

crate::export_service!(SearchServer, SearchController, auth = UserPermissions::USER);
