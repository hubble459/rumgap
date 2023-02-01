use migration::Expr;
use sea_orm::{DatabaseConnection, DeriveColumn, EntityTrait, EnumIter, IdenStatic, QuerySelect};
use tonic::{Request, Response, Status};

use crate::proto::meta_server::{Meta, MetaServer};
use crate::proto::{Empty, MetaReply};

#[derive(Debug, Default)]
pub struct MyMeta {}

#[derive(Copy, Clone, Debug, EnumIter, DeriveColumn)]
enum QueryAs {
    Strings,
}

#[tonic::async_trait]
impl Meta for MyMeta {
    async fn all(&self, req: Request<Empty>) -> Result<Response<MetaReply>, Status> {
        let db = req.extensions().get::<DatabaseConnection>().unwrap();


        // TODO: Get searchable hostnames from MANGA-PARSER not this
        let hostnames: Vec<String> = entity::manga::Entity::find()
            .select_only()
            .column_as(
                Expr::cust("distinct (regexp_matches(manga.url, '://([^/]+)'))[1]"),
                QueryAs::Strings,
            )
            .into_values::<_, QueryAs>()
            .all(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let tags: Vec<String> = entity::manga::Entity::find()
            .select_only()
            .column_as(
                Expr::cust("distinct unnest(manga.genres)"),
                QueryAs::Strings,
            )
            .into_values::<_, QueryAs>()
            .all(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(MetaReply {
            tags,
            hostnames,
        }))
    }
}

crate::export_service!(MetaServer, MyMeta);
