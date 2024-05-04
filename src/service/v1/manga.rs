use std::pin::Pin;
use std::time::Duration;

use chrono::{NaiveDateTime, Utc};
use futures::Stream;
use manga_parser::scraper::MangaScraper;
use manga_parser::Url;
use migration::{Expr, IntoCondition, JoinType, OnConflict};
use sea_orm::ActiveValue::{NotSet, Set};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, DeriveColumn, EntityTrait, EnumIter,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait, RelationTrait, Select,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status};

use crate::proto::manga_server::{Manga, MangaServer};
use crate::proto::{
    Id, MangaReply, MangaRequest, MangasReply, MangasRequest, PaginateReply, PaginateSearchQuery,
};
use crate::util::auth::Authorize;
use crate::util::db::DatabaseRequest;
use crate::util::scrape_error_proto::StatusWrapper;
use crate::util::search::manga::lucene_filter;
use crate::{data, util, MANGA_PARSER};

type ResponseStream = Pin<Box<dyn Stream<Item = Result<MangaReply, Status>> + Send>>;

#[derive(Copy, Clone, Debug, EnumIter, DeriveColumn)]
enum MangaOnlyUrlAndId {
    Id,
    Url,
}

pub const NEXT_UPDATE_QUERY: &str =
    "(MAX(chapter.posted) + (MAX(chapter.posted) - MIN(chapter.posted)) / NULLIF(COUNT(*) - 1, 0))";

/// Get a "full" manga by it's ID
#[rustfmt::skip]
pub async fn get_manga_by_id(db: &DatabaseConnection, logged_in: Option<&entity::user::Model>, manga_id: i32) -> Result<MangaReply, Status> {
    use entity::chapter::Column as ChapterColumn;

    let manga = entity::manga::Entity::find_by_id(manga_id)
        .left_join(entity::chapter::Entity)
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
                                .into_condition()
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
        .map_err(|e| Status::internal(e.to_string()))?
        .ok_or(Status::not_found("Manga not found"))?;

    Ok(manga.into())
}

/// Save/ refresh and get a manga
pub async fn save_manga(
    db: &DatabaseConnection,
    logged_in: Option<&entity::user::Model>,
    id: Option<i32>,
    url: Url,
) -> Result<MangaReply, Status> {
    info!("Saving manga [{}]", url.to_string());

    // TODO: backtick and probably other special characters
    // TODO: should be replaced with normal characters
    let manga: manga_parser::model::Manga = MANGA_PARSER
        .manga(&url)
        .await
        .map_err(StatusWrapper::from)?;

    let saved = entity::manga::ActiveModel {
        id: id.map_or(NotSet, Set),
        title: Set(manga.title),
        description: Set(manga.description),
        is_ongoing: Set(manga.is_ongoing),
        cover: Set(manga.cover_url.map(|url| url.to_string())),
        url: Set(manga.url.to_string()),
        authors: Set(manga.authors),
        alt_titles: Set(manga.alternative_titles),
        genres: Set(manga.genres),
        status: manga.status.map_or(NotSet, Set),
        ..Default::default()
    }
    .save(db)
    .await
    .map_err(|e| Status::internal(e.to_string()))?;

    let manga_id = saved.id.unwrap();

    if manga.chapters.is_empty() {
        error!(
            "No chapters found for {} [{}]",
            manga_id,
            manga.url.to_string()
        );
    } else {
        if id.is_some() {
            let count_chapters = entity::chapter::Entity::find()
                .filter(entity::chapter::Column::MangaId.eq(manga_id))
                .count(db)
                .await
                .map_err(|e| Status::internal(e.to_string()))?;
            if (manga.chapters.len() as u64) < count_chapters {
                // If there are suddenly less chapters than we have in our database, we reset our chapters
                // Remove old chapters
                let res = entity::chapter::Entity::delete_many()
                    .filter(entity::chapter::Column::MangaId.eq(manga_id))
                    .exec(db)
                    .await
                    .map_err(|e| Status::internal(e.to_string()))?;
                info!("Cleared {} chapter(s)", res.rows_affected);
            }
        }

        // Add new chapters
        let mut chapters = vec![];
        for chapter in manga.chapters.iter().rev() {
            chapters.push(entity::chapter::ActiveModel {
                manga_id: Set(manga_id),
                number: Set(chapter.number),
                url: Set(chapter.url.to_string()),
                title: Set(chapter.title.clone()),
                posted: Set(chapter.date.map(|date| date.into())),
                ..Default::default()
            });
        }
        info!("Inserting {} chapter(s)", chapters.len());
        // Insert all in batch
        let res = entity::chapter::Entity::insert_many(chapters)
            .on_conflict(
                OnConflict::column(entity::chapter::Column::Url)
                    .do_nothing()
                    .to_owned(),
            )
            .exec_without_returning(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        info!("Inserted {} unique chapter(s)", res);
    }

    get_manga_by_id(db, logged_in, manga_id).await
}

pub fn index_manga(logged_in: Option<entity::user::Model>) -> Select<entity::manga::Entity> {
    entity::manga::Entity::find()
        .left_join(entity::chapter::Entity)
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
                    entity::reading::Relation::Manga.def().rev().on_condition(
                        move |_left, right| {
                            Expr::col((right, entity::reading::Column::UserId))
                                .eq(user_id)
                                .into_condition()
                        },
                    ),
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
        let logged_in =
            request
                .extensions()
                .get::<entity::user::Model>()
                .ok_or(Status::permission_denied(
                    "You can only add a manga if you are logged in",
                ))?;
        let req = request.get_ref();
        let url = &req.url;

        let existing = entity::manga::Entity::find()
            .filter(entity::manga::Column::Url.eq(url.clone()))
            .one(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        if existing.is_some() {
            return Err(Status::already_exists(
                "Manga with this url already exists!",
            ));
        }

        let url = Url::parse(url).map_err(|e| Status::invalid_argument(e.to_string()))?;

        Ok(Response::new(
            save_manga(db, Some(logged_in), None, url).await?,
        ))
    }

    /// Create multiple manga
    async fn create_many(
        &self,
        request: Request<MangasRequest>,
    ) -> Result<Response<Self::CreateManyStream>, Status> {
        let db = request
            .extensions()
            .get::<DatabaseConnection>()
            .unwrap()
            .clone();
        let logged_in = request
            .extensions()
            .get::<entity::user::Model>()
            .ok_or(Status::permission_denied(
                "You can only add a manga if you are logged in",
            ))?
            .clone();
        let req = request.get_ref();
        let mut stream =
            Box::pin(tokio_stream::iter(req.urls.clone()).throttle(Duration::from_millis(200)));

        // spawn and channel are required if you want handle "disconnect" functionality
        // the `out_stream` will not be polled after client disconnect
        let (tx, rx) = mpsc::channel(128);
        tokio::spawn(async move {
            while let Some(url) = stream.next().await {
                let url = Url::parse(&url).map_err(|e| Status::invalid_argument(e.to_string()));

                let res: Result<MangaReply, Status> = match url {
                    Ok(url) => save_manga(&db, Some(&logged_in), None, url).await,
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
        Ok(Response::new(
            Box::pin(output_stream) as Self::CreateManyStream
        ))
    }

    /// Get one manga
    async fn get(&self, request: Request<Id>) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize().ok();
        let req = request.get_ref();
        let manga_id = req.id;

        let (url, updated_at): (String, NaiveDateTime) =
            entity::manga::Entity::find_by_id(manga_id)
                .select_only()
                .column(entity::manga::Column::Url)
                .column(entity::manga::Column::UpdatedAt)
                .into_values::<_, data::manga::Minimal>()
                .one(db)
                .await
                .map_err(|e| Status::internal(e.to_string()))?
                .ok_or(Status::not_found("Manga not found"))?;

        let interval_ms: i64 = std::env::var("MANGA_UPDATE_INTERVAL_MS")
            .unwrap_or("3600000".to_string())
            .parse()
            .unwrap_or(3600000);

        // Check if it should be updated
        let manga = if (Utc::now().naive_utc() - chrono::Duration::milliseconds(interval_ms))
            > updated_at
        {
            // Update
            info!("Updating manga with id '{}' [{}]", manga_id, url);
            save_manga(db, logged_in, Some(manga_id), Url::parse(&url).unwrap()).await?
        } else {
            get_manga_by_id(db, logged_in, manga_id).await?
        };

        Ok(Response::new(manga))
    }

    /// Force update a manga
    async fn update(&self, request: Request<Id>) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize().ok();
        let req = request.get_ref();
        let manga_id = req.id;

        let (_id, url): (i32, String) = entity::manga::Entity::find_by_id(manga_id)
            .select_only()
            .columns([entity::manga::Column::Id, entity::manga::Column::Url])
            .into_values::<_, MangaOnlyUrlAndId>()
            .one(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or(Status::not_found("Manga not found"))?;

        info!("Updating manga with id '{}' [{}]", manga_id, url);
        let manga = save_manga(db, logged_in, Some(manga_id), Url::parse(&url).unwrap()).await?;

        Ok(Response::new(manga))
    }

    /// Find or create a manga by URL
    async fn find_or_create(
        &self,
        request: Request<MangaRequest>,
    ) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in =
            request
                .extensions()
                .get::<entity::user::Model>()
                .ok_or(Status::permission_denied(
                    "You can only add a manga if you are logged in",
                ))?;
        let req = request.get_ref();
        let url = &req.url;

        let existing = entity::manga::Entity::find()
            .filter(entity::manga::Column::Url.eq(url.clone()))
            .one(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let url = Url::parse(url).map_err(|e| Status::invalid_argument(e.to_string()))?;

        Ok(Response::new(
            save_manga(db, Some(logged_in), existing.map(|manga| manga.id), url).await?,
        ))
    }

    /// Paginate manga
    async fn index(
        &self,
        request: Request<PaginateSearchQuery>,
    ) -> Result<Response<MangasReply>, Status> {
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

        let paginate = paginate
            .into_model::<data::manga::Full>()
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

        Ok(Response::new(MangasReply {
            pagination: Some(PaginateReply {
                page,
                per_page,
                max_page,
                total: amount.number_of_items,
            }),
            items: items.into_iter().map(|manga| manga.into()).collect(),
        }))
    }

    async fn similar(&self, request: Request<Id>) -> Result<Response<MangasReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize().ok().cloned();

        let id = request.get_ref().id;
        let (manga_title, alt_titles): (String, Vec<String>) =
            entity::manga::Entity::find_by_id(id)
                .select_only()
                .column(entity::manga::Column::Title)
                .column(entity::manga::Column::AltTitles)
                .into_tuple()
                .one(db)
                .await
                .map_err(|e| Status::internal(e.to_string()))?
                .ok_or(Status::not_found("Manga not found"))?;

        let title_matches = alt_titles
            .into_iter()
            .map(|alt_title| {
                Expr::cust_with_values("$1 % any(manga.alt_titles || manga.title)", [alt_title])
            })
            .fold(
                Expr::cust_with_values("$1 % any(manga.alt_titles || manga.title)", [manga_title]),
                |expr, alt_title_expr| expr.or(alt_title_expr),
            );

        let similar = index_manga(logged_in)
            .filter(entity::manga::Column::Id.ne(id).and(title_matches))
            .into_model::<data::manga::Full>()
            .all(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(MangasReply {
            pagination: None,
            items: similar.into_iter().map(|manga| manga.into()).collect(),
        }))
    }
}

crate::export_service!(MangaServer, MangaController);
