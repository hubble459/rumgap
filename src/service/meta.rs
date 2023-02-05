use manga_parser::parser::Parser;
use migration::{Expr, IntoCondition, JoinType};
use sea_orm::{
    DatabaseConnection, DeriveColumn, EntityTrait, EnumIter, QuerySelect,
    RelationTrait, Select,
};
use tonic::{Request, Response, Status};

use super::manga::MANGA_PARSER;
use crate::interceptor::auth::LoggedInUser;
use crate::proto::meta_server::{Meta, MetaServer};
use crate::proto::{
    MetaGenresOption, MetaGenresRequest, MetaHostnamesOption, MetaHostnamesRequest, MetaReply,
};

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
    logged_in: Option<LoggedInUser>,
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
                .can_search()
                .unwrap_or_default()
                .iter()
                .map(|url| url.to_string())
                .collect(),
        },
    };

    Ok(reply)
}

#[tonic::async_trait]
impl Meta for MyMeta {
    async fn genres(&self, req: Request<MetaGenresRequest>) -> Result<Response<MetaReply>, Status> {
        let db = req.extensions().get::<DatabaseConnection>().unwrap();
        let logged_in = req.extensions().get::<LoggedInUser>().cloned();
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
        let logged_in = req.extensions().get::<LoggedInUser>().cloned();
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
}

crate::export_service!(MetaServer, MyMeta);
