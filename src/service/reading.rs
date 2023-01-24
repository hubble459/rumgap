use migration::Expr;
use sea_orm::ActiveValue::{Set};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, IntoActiveModel,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, RelationTrait,
};
use tonic::{Request, Response, Status};

use super::manga::NEXT_UPDATE_QUERY;
use crate::interceptor::auth::{LoggedInUser, UserPermissions};
use crate::proto::reading_server::{Reading, ReadingServer};
use crate::proto::{
    Empty, Id,
    PaginateReply, PaginateSearchQuery, ReadingPatchRequest, ReadingPostRequest, ReadingReply,
    ReadingsReply,
};
use crate::util::search::manga::lucene_filter;
use crate::{data, util};

#[rustfmt::skip]
pub async fn get_reading_by_id(db: &DatabaseConnection, user_id: i32, manga_id: i32) -> Result<ReadingReply, Status> {
    let (reading, manga) = entity::reading::Entity::find_by_id((user_id, manga_id))
        .left_join(entity::manga::Entity)
        .select_also(entity::manga::Entity)
        .join(
            migration::JoinType::LeftJoin,
            entity::manga::Relation::Chapter.def(),
        )
        .column_as(entity::chapter::Column::Id.count(), "B_count_chapters")
        .column_as(entity::chapter::Column::Posted.max(), "B_last")
        .column_as(Expr::cust(NEXT_UPDATE_QUERY), "B_next")
        .group_by(entity::manga::Column::Id)
        .group_by(entity::reading::Column::MangaId)
        .into_model::<data::reading::Full, data::manga::Full>()
        .one(db)
        .await
        .map_err(|e| Status::internal(e.to_string()))?
        .ok_or(Status::not_found("Chapter not found"))?;

    Ok(ReadingReply {
        id: reading.id,
        title: reading.title,
        progress: reading.progress,
        cover: reading.cover,
        count_chapters: reading.count_chapters,
        manga: manga.map(|m| m.into()),
        created_at: reading.created_at.timestamp_millis(),
        updated_at: reading.updated_at.timestamp_millis(),
    })
}

#[derive(Debug, Default)]
pub struct MyReading {}

#[tonic::async_trait]
impl Reading for MyReading {
    async fn get(&self, request: Request<Id>) -> Result<Response<ReadingReply>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let logged_in = request.extensions().get::<LoggedInUser>().unwrap();
        let req = request.get_ref();

        Ok(Response::new(
            get_reading_by_id(db, logged_in.id, req.id).await?,
        ))
    }

    async fn index(
        &self,
        request: Request<PaginateSearchQuery>,
    ) -> Result<Response<ReadingsReply>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let logged_in = request.extensions().get::<LoggedInUser>().unwrap();
        let req = request.get_ref();
        let per_page = req.per_page.unwrap_or(10).clamp(1, 50);

        // Create paginate object
        let mut paginate = entity::reading::Entity::find()
            .filter(entity::reading::Column::UserId.eq(logged_in.id))
            .left_join(entity::manga::Entity)
            .select_also(entity::manga::Entity)
            .join(
                migration::JoinType::LeftJoin,
                entity::manga::Relation::Chapter.def(),
            )
            .column_as(entity::chapter::Column::Id.count(), "B_count_chapters")
            .column_as(entity::chapter::Column::Posted.max(), "B_last")
            .column_as(Expr::cust(NEXT_UPDATE_QUERY), "B_next")
            .group_by(entity::manga::Column::Id)
            .group_by(entity::reading::Column::MangaId);

        if let Some(search) = req.search.clone() {
            paginate = paginate.having(lucene_filter(search.into())?);
        }

        if let Some(order) = req.order.clone() {
            let columns = util::order::reading::parse(&order)?;
            for (column, order) in columns {
                paginate = paginate.order_by(column, order);
            }
        }

        let paginate = paginate
            .into_model::<data::reading::Full, data::manga::Full>()
            .paginate(db, per_page);

        // Get max page and total items
        let amount = paginate
            .num_items_and_pages()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let page = req.page.unwrap_or(0).clamp(0, amount.number_of_pages - 1);

        // Get items from page
        let items = paginate
            .fetch_page(page)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(ReadingsReply {
            pagination: Some(PaginateReply {
                page,
                per_page,
                max_page: amount.number_of_pages - 1,
                total: amount.number_of_items,
            }),
            items: items
                .into_iter()
                .map(|(reading, manga)| ReadingReply {
                    id: reading.id,
                    title: reading.title,
                    progress: reading.progress,
                    cover: reading.cover,
                    count_chapters: reading.count_chapters,
                    manga: manga.map(|m| m.into()),
                    created_at: reading.created_at.timestamp_millis(),
                    updated_at: reading.updated_at.timestamp_millis(),
                })
                .collect(),
        }))
    }

    async fn edit(
        &self,
        request: Request<ReadingPatchRequest>,
    ) -> Result<Response<ReadingReply>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let logged_in = request.extensions().get::<LoggedInUser>().unwrap();
        let req = request.get_ref();

        let mut reading = entity::reading::Entity::find_by_id((logged_in.id, req.id))
            .one(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or(Status::not_found("Reading not found"))?
            .into_active_model();

        reading.progress = Set(req.progress);
        let reading = reading
            .update(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(
            get_reading_by_id(db, logged_in.id, reading.manga_id).await?,
        ))
    }

    async fn create(
        &self,
        request: Request<ReadingPostRequest>,
    ) -> Result<Response<ReadingReply>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let logged_in = request.extensions().get::<LoggedInUser>().unwrap();
        let req = request.get_ref();

        let saved = entity::reading::ActiveModel {
            manga_id: Set(req.manga_id),
            user_id: Set(logged_in.id),
            ..Default::default()
        }
        .insert(db)
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

        let reading = get_reading_by_id(&db, saved.user_id, saved.manga_id).await?;

        Ok(Response::new(reading))
    }

    async fn delete(&self, request: Request<Id>) -> Result<Response<Empty>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let logged_in = request.extensions().get::<LoggedInUser>().unwrap();
        let req = request.get_ref();

        // Delete reading
        let reading = entity::reading::Entity::delete_by_id((logged_in.id, req.id))
            .exec(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        // Check if deleted
        if reading.rows_affected == 0 {
            Err(Status::not_found("Reading not found"))
        } else {
            Ok(Response::new(Empty::default()))
        }
    }
}

crate::export_server!(ReadingServer, MyReading, auth = UserPermissions::USER);
