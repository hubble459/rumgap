use std::pin::Pin;
use std::time::Duration;

use futures::Stream;
use manga_parser::parser::{MangaParser, Parser};
use manga_parser::Url;
use migration::Expr;
use sea_orm::ActiveValue::{NotSet, Set};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QuerySelect,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status};

use crate::data;
use crate::proto::manga_server::{Manga, MangaServer};
use crate::proto::{Id, MangaReply, MangaRequest, MangasReply, MangasRequest, PaginateQuery};

type ResponseStream = Pin<Box<dyn Stream<Item = Result<MangaReply, Status>> + Send>>;

lazy_static! {
    pub static ref MANGA_PARSER: MangaParser = MangaParser::new();
}

pub const NEXT_UPDATE_QUERY: &str =
    "(MAX(chapter.posted) + (MAX(chapter.posted) - MIN(chapter.posted)) / NULLIF(COUNT(*) - 1, 0))";

#[rustfmt::skip]
pub async fn get_manga_by_id(db: &DatabaseConnection, manga_id: i32) -> Result<MangaReply, Status> {
    let manga = entity::manga::Entity::find_by_id(manga_id)
        .left_join(entity::chapter::Entity)
        .column_as(entity::chapter::Column::Id.count(), "count_chapters")
        .column_as(entity::chapter::Column::Posted.max(), "last")
        .column_as(Expr::cust(NEXT_UPDATE_QUERY), "next")
        .group_by(entity::manga::Column::Id)
        .into_model::<data::manga::Full>()
        .one(db)
        .await
        .map_err(|e| Status::internal(e.to_string()))?
        .ok_or(Status::not_found("Manga not found"))?;

    Ok(manga.into())
}

pub async fn save_manga(
    db: &DatabaseConnection,
    id: Option<i32>,
    url: Url,
) -> Result<MangaReply, Status> {
    info!("Saving manga [{}]", url.to_string());

    let m = MANGA_PARSER
        .manga(url)
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

    let saved = entity::manga::ActiveModel {
        id: id.map_or(NotSet, |id| Set(id)),
        title: Set(m.title),
        description: Set(m.description),
        is_ongoing: Set(m.is_ongoing),
        cover: Set(m.cover.map(|url| url.to_string())),
        url: Set(m.url.to_string()),
        authors: Set(m.authors),
        alt_titles: Set(m.alt_titles),
        genres: Set(m.genres),
        ..Default::default()
    }
    .save(db)
    .await
    .map_err(|e| Status::internal(e.to_string()))?;

    let manga_id = saved.id.unwrap();

    if m.chapters.is_empty() {
        error!("No chapters found for {} [{}]", manga_id, m.url.to_string());
    } else {
        // Remove old chapters
        let res = entity::chapter::Entity::delete_many()
            .filter(entity::chapter::Column::MangaId.eq(manga_id))
            .exec(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        if id.is_some() {
            info!("Cleared {} chapter(s)", res.rows_affected);
        }

        // Add new chapters
        let mut chapters = vec![];
        for chapter in m.chapters.iter() {
            chapters.push(entity::chapter::ActiveModel {
                manga_id: Set(manga_id),
                number: Set(chapter.number),
                url: Set(chapter.url.to_string()),
                title: Set(chapter.title.clone()),
                posted: Set(chapter.posted.map(|date| date.into())),
                ..Default::default()
            });
        }
        info!("Inserting {} chapter(s)", chapters.len());
        // Insert all in batch
        entity::chapter::Entity::insert_many(chapters)
            .exec_without_returning(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
    }

    get_manga_by_id(db, manga_id).await
}

#[derive(Debug, Default)]
pub struct MyManga {}

#[tonic::async_trait]
impl Manga for MyManga {
    type CreateManyStream = ResponseStream;

    async fn create(&self, request: Request<MangaRequest>) -> Result<Response<MangaReply>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let req = request.get_ref();

        let url = Url::parse(&req.url)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        Ok(Response::new(
            save_manga(db, None, url).await?
        ))
    }

    async fn create_many(
        &self,
        request: Request<MangasRequest>,
    ) -> Result<Response<Self::CreateManyStream>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap().clone();
        let req = request.get_ref();
        let mut stream =
            Box::pin(tokio_stream::iter(req.items.clone()).throttle(Duration::from_millis(200)));

        // spawn and channel are required if you want handle "disconnect" functionality
        // the `out_stream` will not be polled after client disconnect
        let (tx, rx) = mpsc::channel(128);
        tokio::spawn(async move {
            while let Some(item) = stream.next().await {
                let url =
                    Url::parse(&item.url).map_err(|e| Status::invalid_argument(e.to_string()));

                let res: Result<MangaReply, Status> = match url {
                    Ok(url) => save_manga(&db, None, url).await,
                    Err(e) => Err(e),
                };

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

    async fn get(&self, request: Request<Id>) -> Result<Response<MangaReply>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let req = request.get_ref();
        
        Ok(Response::new(get_manga_by_id(db, req.id).await?))
    }

    async fn index(
        &self,
        request: Request<PaginateQuery>,
    ) -> Result<Response<MangasReply>, Status> {
        unimplemented!()
    }
}

crate::export_server!(MangaServer, MyManga);
