use std::collections::HashMap;
use std::pin::Pin;
use std::time::Duration;

use chrono::{NaiveDateTime, Utc};
use futures::Stream;
use manga_parser::scraper::MangaScraper;
use manga_parser::Url;
use migration::{Expr, ExprTrait, JoinType, OnConflict};
use sea_orm::ActiveValue::{NotSet, Set};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, QueryTrait, RelationTrait, Select,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status};

use crate::proto::manga_server::{Manga, MangaServer};
use crate::proto::{
    AddSourceRequest, BackfillStatusReply, Empty, Id, MangaReply, MangaRequest, MangaSourceReply, MangasReply,
    MangasRequest, PaginateReply, PaginateSearchQuery, RemoveSourceRequest, SetPrimarySourceRequest,
};
use crate::util::auth::Authorize;
use crate::util::backfill;
use crate::util::db::DatabaseRequest;
use crate::util::scrape_error_proto::StatusWrapper;
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

/// Restricts a manga -> manga_source join to just the primary source, since count_chapters/
/// last/next are computed from the primary source only.
fn join_primary_source_and_chapters(query: Select<entity::manga::Entity>) -> Select<entity::manga::Entity> {
    query
        .join(
            JoinType::LeftJoin,
            entity::manga::Relation::MangaSource.def().on_condition(|_left, right| {
                Expr::col((right, entity::manga_source::Column::IsPrimary))
                    .eq(true)
                    .into()
            }),
        )
        .join(JoinType::LeftJoin, entity::manga_source::Relation::Chapter.def())
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

    let mut map: HashMap<i32, Vec<MangaSourceReply>> = HashMap::new();
    for source in sources {
        map.entry(source.manga_id).or_default().push(source.into());
    }
    Ok(map)
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
#[rustfmt::skip]
pub async fn get_manga_by_id(db: &DatabaseConnection, logged_in: Option<&entity::user::Model>, manga_id: i32) -> Result<MangaReply, Status> {
    use entity::chapter::Column as ChapterColumn;

    let manga = join_primary_source_and_chapters(entity::manga::Entity::find_by_id(manga_id))
        .column_as(ChapterColumn::Id.count(), "count_chapters")
        .column_as(ChapterColumn::Posted.max(), "last")
        .column_as(Expr::cust(NEXT_UPDATE_QUERY), "next")
        .group_by(entity::manga::Column::Id)
        .column_as(Expr::cust("null"), "progress")
        .apply_if(logged_in, |query, logged_in| {
            let user_id = logged_in.id;
            query
                .join(
                JoinType::LeftJoin,
                entity::reading::Relation::Manga.def().rev().on_condition(
                        move |_left, right| {
                            Expr::col((right, entity::reading::Column::UserId))
                                .eq(user_id)
                                .into()
                        },
                    ),
                )
                .column_as(entity::reading::Column::Progress, "progress")
                .group_by(entity::reading::Column::UserId)
                .group_by(entity::reading::Column::MangaId)
        })
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
/// to *some* canonical_chapter this way - `canonical_chapter_id` is left NULL only for the
/// rare within-this-scrape ambiguous case (two of this source's own new chapters would
/// claim the same ordinal - can't both be right, so neither is guessed; left for manual
/// `LinkChapter` resolution instead) since `canonical_chapter(manga_id, ordinal)` is unique
/// and can't hold two rows for the same ordinal anyway.
async fn match_canonical_chapters(
    db: &DatabaseConnection,
    manga_id: i32,
    chapters: &[entity::chapter::Model],
) -> Result<(), Status> {
    use std::collections::HashSet;

    let mut claimed_ordinals: HashSet<sea_orm::prelude::Decimal> = HashSet::new();

    for chapter in chapters {
        let ordinal = sea_orm::prelude::Decimal::from_f32_retain(chapter.number)
            .unwrap_or_default()
            .round_dp(3);

        if !claimed_ordinals.insert(ordinal) {
            warn!(
                "Ambiguous canonical ordinal {} for manga {} (chapter {}, ordinal already claimed \
                 by another chapter in this same scrape) - leaving unlinked for manual resolution",
                ordinal, manga_id, chapter.id
            );
            continue;
        }

        let existing = entity::canonical_chapter::Entity::find()
            .filter(entity::canonical_chapter::Column::MangaId.eq(manga_id))
            .filter(entity::canonical_chapter::Column::Ordinal.eq(ordinal))
            .one(db)
            .await
            .map_err(internal)?;

        let canonical_id = match existing {
            Some(existing) => existing.id,
            None => {
                entity::canonical_chapter::ActiveModel {
                    manga_id: Set(manga_id),
                    ordinal: Set(ordinal),
                    ..Default::default()
                }
                .insert(db)
                .await
                .map_err(internal)?
                .id
            }
        };

        entity::chapter::ActiveModel {
            id: Set(chapter.id),
            canonical_chapter_id: Set(Some(canonical_id)),
            ..Default::default()
        }
        .update(db)
        .await
        .map_err(internal)?;
    }

    Ok(())
}

/// Insert/reconcile chapters scraped for a single manga_source, then run the canonical
/// matching heuristic on exactly the newly-inserted rows (never touching chapters that
/// already have a link, whether auto-matched before or manually unlinked).
async fn sync_chapters_for_source(
    db: &DatabaseConnection,
    manga_id: i32,
    manga_source_id: i32,
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
        // If there are suddenly less chapters than we have in our database, we reset our chapters
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

/// Create a brand-new canonical manga from a freshly-scraped URL, with this as its (first,
/// and therefore primary) source.
pub async fn create_manga(db: &DatabaseConnection, url: Url) -> Result<i32, Status> {
    info!("Creating manga [{}]", url);

    let manga: manga_parser::model::Manga = MANGA_PARSER.manga(&url).await.map_err(StatusWrapper::from)?;

    let saved_manga = entity::manga::ActiveModel {
        title: Set(manga.title),
        description: Set(manga.description),
        is_ongoing: Set(manga.is_ongoing),
        cover_source_url: Set(manga.cover_url.map(|url| url.to_string())),
        authors: Set(manga.authors),
        alt_titles: Set(manga.alternative_titles),
        genres: Set(manga.genres),
        status: manga.status.map_or(NotSet, Set),
        ..Default::default()
    }
    .insert(db)
    .await
    .map_err(internal)?;

    let saved_source = entity::manga_source::ActiveModel {
        manga_id: Set(saved_manga.id),
        url: Set(manga.url.to_string()),
        hostname: Set(hostname_of(&manga.url)),
        is_primary: Set(true),
        ..Default::default()
    }
    .insert(db)
    .await
    .map_err(internal)?;

    sync_chapters_for_source(db, saved_manga.id, saved_source.id, &manga.chapters).await?;

    Ok(saved_manga.id)
}

/// Add a new (by default, secondary) source to an existing manga: scrapes via the existing
/// call path, creates the manga_source row, inserts chapters against it, runs the matching
/// heuristic. Only the *primary* source's scrape updates canonical manga fields, so a brand
/// new non-primary source never touches them.
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

    let manga: manga_parser::model::Manga = MANGA_PARSER.manga(&url).await.map_err(StatusWrapper::from)?;

    let saved_source = entity::manga_source::ActiveModel {
        manga_id: Set(manga_id),
        url: Set(manga.url.to_string()),
        hostname: Set(hostname_of(&manga.url)),
        is_primary: Set(false),
        ..Default::default()
    }
    .insert(db)
    .await
    .map_err(internal)?;

    sync_chapters_for_source(db, manga_id, saved_source.id, &manga.chapters).await?;

    Ok(saved_source.id)
}

/// Refresh an existing manga_source in place (re-scrape its url). If it's the primary
/// source, canonical manga.title/description/cover/etc are updated too - a secondary
/// source's refresh must never stomp canonical metadata.
pub async fn refresh_manga_source(db: &DatabaseConnection, manga_source_id: i32) -> Result<i32, Status> {
    let source = entity::manga_source::Entity::find_by_id(manga_source_id)
        .one(db)
        .await
        .map_err(internal)?
        .ok_or(Status::not_found("Manga source not found"))?;

    let url = Url::parse(&source.url).map_err(|e| Status::invalid_argument(e.to_string()))?;
    info!("Refreshing manga source {} [{}]", manga_source_id, url);

    let manga: manga_parser::model::Manga = MANGA_PARSER.manga(&url).await.map_err(StatusWrapper::from)?;

    if source.is_primary {
        entity::manga::ActiveModel {
            id: Set(source.manga_id),
            title: Set(manga.title.clone()),
            description: Set(manga.description.clone()),
            is_ongoing: Set(manga.is_ongoing),
            // Only overwrite the cover when this scrape actually found one - cover_url is a
            // genuinely optional field in manga_parser's model (unlike title/description,
            // which are required on the builder and fail the whole scrape if unextractable),
            // so a successful-but-cover-less scrape must not silently wipe a previously-good
            // cover with NULL.
            cover_source_url: manga.cover_url.clone().map_or(NotSet, |url| Set(Some(url.to_string()))),
            authors: Set(manga.authors.clone()),
            alt_titles: Set(manga.alternative_titles.clone()),
            genres: Set(manga.genres.clone()),
            status: manga.status.clone().map_or(NotSet, Set),
            ..Default::default()
        }
        .update(db)
        .await
        .map_err(internal)?;
    }

    sync_chapters_for_source(db, source.manga_id, manga_source_id, &manga.chapters).await?;

    Ok(source.manga_id)
}

/// Reuses the already-live pg_trgm `similar()` matching (title/alt_titles fuzzy match) to
/// suggest "is this the same manga you already have?" for a title that isn't tracked
/// locally yet (e.g. a fresh search result) - used by `SearchManga.suggested_manga_id`.
pub async fn find_suggested_manga_id(db: &DatabaseConnection, title: &str) -> Result<Option<i32>, Status> {
    entity::manga::Entity::find()
        .filter(Expr::cust_with_values(
            "$1 % any(manga.alt_titles || manga.title)",
            [title.to_string()],
        ))
        .select_only()
        .column(entity::manga::Column::Id)
        .into_tuple::<i32>()
        .one(db)
        .await
        .map_err(internal)
}

pub fn index_manga(logged_in: Option<entity::user::Model>) -> Select<entity::manga::Entity> {
    join_primary_source_and_chapters(entity::manga::Entity::find())
        .column_as(entity::chapter::Column::Id.count(), "count_chapters")
        .column_as(entity::chapter::Column::Posted.max(), "last")
        .column_as(Expr::cust(NEXT_UPDATE_QUERY), "next")
        .group_by(entity::manga::Column::Id)
        .column_as(Expr::cust("null"), "progress")
        .apply_if(logged_in, |query, logged_in| {
            let user_id = logged_in.id;
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
                .column_as(entity::reading::Column::Progress, "progress")
                .group_by(entity::reading::Column::MangaId)
                .group_by(entity::reading::Column::UserId)
        })
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
    async fn get(&self, request: Request<Id>) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize().ok();
        let req = request.get_ref();
        let manga_id = req.id;

        let (updated_at,): (NaiveDateTime,) = entity::manga::Entity::find_by_id(manga_id)
            .select_only()
            .column(entity::manga::Column::UpdatedAt)
            .into_values::<_, data::manga::Minimal>()
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("Manga not found"))?;

        let interval_ms: i64 = std::env::var("MANGA_UPDATE_INTERVAL_MS")
            .unwrap_or("3600000".to_string())
            .parse()
            .unwrap_or(3600000);

        // Check if it should be updated
        if (Utc::now().naive_utc() - chrono::Duration::milliseconds(interval_ms)) > updated_at {
            if let Some(primary_source_id) = find_primary_source_id(db, manga_id).await? {
                info!(
                    "Updating manga with id '{}' (primary source {})",
                    manga_id, primary_source_id
                );
                refresh_manga_source(db, primary_source_id).await?;
            }
        }

        Ok(Response::new(get_manga_by_id(db, logged_in, manga_id).await?))
    }

    /// Force update a manga - refreshes its primary source (only a primary-source refresh
    /// updates canonical manga.title/description/cover/etc).
    async fn update(&self, request: Request<Id>) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize().ok();
        let req = request.get_ref();
        let manga_id = req.id;

        let primary_source_id = find_primary_source_id(db, manga_id)
            .await?
            .ok_or(Status::not_found("Manga has no primary source to refresh"))?;

        info!(
            "Updating manga with id '{}' (primary source {})",
            manga_id, primary_source_id
        );
        refresh_manga_source(db, primary_source_id).await?;

        Ok(Response::new(get_manga_by_id(db, logged_in, manga_id).await?))
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
            refresh_manga_source(db, existing.id).await?
        } else {
            let url = Url::parse(url).map_err(|e| Status::invalid_argument(e.to_string()))?;
            create_manga(db, url).await?
        };

        Ok(Response::new(get_manga_by_id(db, Some(&logged_in), manga_id).await?))
    }

    /// Paginate manga
    async fn index(&self, request: Request<PaginateSearchQuery>) -> Result<Response<MangasReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize().ok().cloned();
        let req = request.get_ref();
        let per_page = req.per_page.unwrap_or(10).clamp(1, 50);
        let mut paginate = index_manga(logged_in);

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
            paginate = paginate.order_by(entity::manga::Column::Title, migration::Order::Asc);
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

    async fn similar(&self, request: Request<Id>) -> Result<Response<MangasReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize().ok().cloned();

        let id = request.get_ref().id;
        let (manga_title, alt_titles): (String, Vec<String>) = entity::manga::Entity::find_by_id(id)
            .select_only()
            .column(entity::manga::Column::Title)
            .column(entity::manga::Column::AltTitles)
            .into_tuple()
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("Manga not found"))?;

        let title_matches = alt_titles
            .into_iter()
            .map(|alt_title| Expr::cust_with_values("$1 % any(manga.alt_titles || manga.title)", [alt_title]))
            .fold(
                Expr::cust_with_values("$1 % any(manga.alt_titles || manga.title)", [manga_title]),
                |expr, alt_title_expr| expr.or(alt_title_expr),
            );

        let similar = index_manga(logged_in)
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

    /// Remove a source from a manga. If it's the primary source and another source
    /// still exists, that other source is auto-promoted to primary first so canonical
    /// manga metadata is never left without a primary source backing it.
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
            let replacement = entity::manga_source::Entity::find()
                .filter(entity::manga_source::Column::MangaId.eq(source.manga_id))
                .filter(entity::manga_source::Column::Id.ne(source.id))
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
                    "Auto-promoted manga_source {} to primary after removing primary source {}",
                    replacement.id, source.id
                );
            }
        }

        let manga_id = source.manga_id;
        entity::manga_source::Entity::delete_by_id(req.manga_source_id)
            .exec(db)
            .await
            .map_err(internal)?;

        Ok(Response::new(get_manga_by_id(db, Some(&logged_in), manga_id).await?))
    }

    /// Change which source is considered primary for a manga (the only source whose
    /// refreshes update canonical manga.title/description/cover/etc, and whose chapters
    /// back top-level count_chapters/last/next).
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

    /// Id = manga_source_id. Kicks off (or resumes) a throttled background walk
    /// downloading every not-yet-cached page of every chapter of that source.
    async fn backfill_images(&self, request: Request<Id>) -> Result<Response<Empty>, Status> {
        let db = request.db()?.clone();
        let manga_source_id = request.get_ref().id;

        backfill::start_backfill(db, manga_source_id);

        Ok(Response::new(Empty::default()))
    }

    /// Id = manga_source_id (see BackfillImages). Cheap `GROUP BY status` progress
    /// count for the client to poll on its own schedule.
    async fn get_backfill_status(&self, request: Request<Id>) -> Result<Response<BackfillStatusReply>, Status> {
        let db = request.db()?;
        let manga_source_id = request.get_ref().id;

        let (images_downloaded, images_total) = backfill::backfill_status(db, manga_source_id).await?;

        Ok(Response::new(BackfillStatusReply {
            images_downloaded,
            images_total,
        }))
    }
}

async fn find_primary_source_id(db: &DatabaseConnection, manga_id: i32) -> Result<Option<i32>, Status> {
    entity::manga_source::Entity::find()
        .filter(entity::manga_source::Column::MangaId.eq(manga_id))
        .filter(entity::manga_source::Column::IsPrimary.eq(true))
        .select_only()
        .column(entity::manga_source::Column::Id)
        .into_tuple::<i32>()
        .one(db)
        .await
        .map_err(internal)
}

crate::export_service!(MangaServer, MangaController);

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

        let manga = entity::manga::ActiveModel {
            title: Set("Test Manga".into()),
            description: Set("".into()),
            is_ongoing: Set(true),
            authors: Set(vec![]),
            alt_titles: Set(vec![]),
            genres: Set(vec![]),
            ..Default::default()
        }
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

        sync_chapters_for_source(&db, manga.id, source_c.id, &[dup_a, dup_b])
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
}
