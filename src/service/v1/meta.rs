use manga_parser::scraper::MangaSearcher;
use migration::{Expr, IntoCondition, JoinType};
use sea_orm::{
    ColumnTrait, DatabaseConnection, DeriveColumn, EntityTrait, EnumIter, QueryFilter, QuerySelect,
    RelationTrait, Select,
};
use tonic::{Request, Response, Status};

use crate::proto::meta_server::{Meta, MetaServer};
use crate::proto::{
    Empty, MetaGenresOption, MetaGenresRequest, MetaHostnamesOption, MetaHostnamesRequest,
    MetaReply, StatsReply,
};
use crate::MANGA_PARSER;

#[derive(Debug, Default)]
pub struct MyMeta {}

#[derive(Copy, Clone, Debug, EnumIter, DeriveColumn)]
enum QueryAs {
    Strings,
}

enum MetaOption {
    Reading,
    Manga,
    Online,
}

async fn get_reply(
    db: &DatabaseConnection,
    query: Select<entity::manga::Entity>,
    logged_in: Option<entity::user::Model>,
    meta_option: MetaOption,
) -> Result<MetaReply, Status> {
    let reply = match meta_option {
        MetaOption::Reading => {
            let logged_in =
                logged_in.ok_or(Status::permission_denied("You have to be logged in!"))?;

            MetaReply {
                items: query
                    .join(
                        JoinType::RightJoin,
                        entity::reading::Relation::Manga.def().rev().on_condition(
                            move |_left, right| {
                                Expr::col((right, entity::reading::Column::UserId))
                                    .eq(logged_in.id)
                                    .into_condition()
                            },
                        ),
                    )
                    .into_values::<_, QueryAs>()
                    .all(db)
                    .await
                    .map_err(|e| Status::internal(e.to_string()))?,
            }
        }
        MetaOption::Manga => MetaReply {
            items: query
                .into_values::<_, QueryAs>()
                .all(db)
                .await
                .map_err(|e| Status::internal(e.to_string()))?,
        },
        MetaOption::Online => MetaReply {
            items: MANGA_PARSER
                .searchable_hostnames()
                .iter()
                .map(|url| url.to_string())
                .collect(),
        },
    };

    Ok(reply)
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveColumn)]
enum StatsCount {
    TotalReading,
    TotalChapters,
    Chapters,
}

#[tonic::async_trait]
impl Meta for MyMeta {
    async fn genres(&self, req: Request<MetaGenresRequest>) -> Result<Response<MetaReply>, Status> {
        let db = req.extensions().get::<DatabaseConnection>().unwrap();
        let logged_in = req.extensions().get::<entity::user::Model>().cloned();
        let request = req.get_ref();
        let query = entity::manga::Entity::find().select_only().column_as(
            Expr::cust("distinct unnest(manga.genres)"),
            QueryAs::Strings,
        );

        Ok(Response::new(
            get_reply(
                db,
                query,
                logged_in,
                match request.option() {
                    MetaGenresOption::GenresReading => MetaOption::Reading,
                    MetaGenresOption::GenresManga => MetaOption::Manga,
                },
            )
            .await?,
        ))
    }

    async fn hostnames(
        &self,
        req: Request<MetaHostnamesRequest>,
    ) -> Result<Response<MetaReply>, Status> {
        let db = req.extensions().get::<DatabaseConnection>().unwrap();
        let logged_in = req.extensions().get::<entity::user::Model>().cloned();
        let request = req.get_ref();
        let query = entity::manga::Entity::find().select_only().column_as(
            Expr::cust("distinct (regexp_matches(manga.url, '://([^/]+)'))[1]"),
            QueryAs::Strings,
        );

        Ok(Response::new(
            get_reply(
                db,
                query,
                logged_in,
                match request.option() {
                    MetaHostnamesOption::HostnamesReading => MetaOption::Reading,
                    MetaHostnamesOption::HostnamesManga => MetaOption::Manga,
                    MetaHostnamesOption::HostnamesOnline => MetaOption::Online,
                },
            )
            .await?,
        ))
    }

    async fn stats(&self, req: Request<Empty>) -> Result<Response<StatsReply>, Status> {
        let logged_in =
            req.extensions()
                .get::<entity::user::Model>()
                .ok_or(Status::unauthenticated(
                    "Missing bearer token! Log in first",
                ))?;
        let db = req.extensions().get::<DatabaseConnection>().unwrap();

        let user_id = logged_in.id;

        let count_reading: i64 = entity::reading::Entity::find()
            .filter(entity::reading::Column::UserId.eq(user_id))
            .select_only()
            .column_as(
                Expr::cust("COUNT(CASE WHEN ((SELECT COUNT(*) FROM chapter WHERE chapter.manga_id = reading.manga_id) = reading.progress) THEN 1 END)"),
                "count_reading",
            )
            .into_tuple()
            .one(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or(Status::unknown("How?"))?;

        let stats: (i64, i64, i64) = entity::reading::Entity::find()
            .filter(entity::reading::Column::UserId.eq(user_id))
            .left_join(entity::manga::Entity)
            .join(JoinType::LeftJoin, entity::manga::Relation::Chapter.def())
            .select_only()
            .column_as(
                Expr::cust("COUNT(DISTINCT reading.manga_id)"),
                StatsCount::TotalReading,
            )
            .column_as(
                Expr::cust("COUNT(DISTINCT chapter.id)"),
                StatsCount::TotalChapters,
            )
            .column_as(
                Expr::cust("SUM(DISTINCT reading.progress)"),
                StatsCount::Chapters,
            )
            .into_values::<_, StatsCount>()
            .one(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or(Status::unknown("How?"))?;

        Ok(Response::new(StatsReply {
            count_total_reading: stats.0,
            count_total_chapters: stats.1,
            count_reading,
            count_chapters: stats.2,
        }))
    }
}

crate::export_service!(MetaServer, MyMeta);
