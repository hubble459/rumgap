use manga_parser::parser::Parser;
use migration::{Expr, IntoCondition, JoinType};
use sea_orm::{
    ColumnTrait, DatabaseConnection, DeriveColumn, EntityTrait, EnumIter, IdenStatic, QueryFilter,
    QuerySelect, RelationTrait,
};
use tonic::{Request, Response, Status};

use super::manga::MANGA_PARSER;
use crate::interceptor::auth::LoggedInUser;
use crate::proto::search_server::{Search, SearchServer};
use crate::proto::{SearchManga, SearchReply, SearchRequest};

#[derive(Debug, Default)]
pub struct MySearch {}

#[derive(Copy, Clone, Debug, EnumIter, DeriveColumn)]
enum QueryAs {
    Id,
    Url,
    Progress,
}

#[tonic::async_trait]
impl Search for MySearch {
    /// Edit reading progress
    async fn manga(
        &self,
        request: Request<SearchRequest>,
    ) -> Result<Response<SearchReply>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let logged_in = request.extensions().get::<LoggedInUser>();
        let req = request.get_ref();

        let search_results = MANGA_PARSER
            .search(req.keyword.clone(), req.hostnames.clone())
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

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
                        Expr::tbl(right, entity::reading::Column::UserId)
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
                        cover: item.cover.map(|cover| cover.to_string()),
                        posted: item.posted.map(|date| date.timestamp_millis()),
                        is_reading: existing
                            .map_or(false, |(_id, _url, progress)| progress.is_some()),
                        manga_id: existing.map(|(id, ..)| id.clone()),
                    }
                })
                .collect(),
        }))
    }
}

crate::export_service!(SearchServer, MySearch);
