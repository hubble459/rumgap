use sea_orm::ActiveValue::{self, Set};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, IntoActiveModel};
use tonic::{Request, Response, Status};

use super::manga::get_manga_by_id;
use crate::interceptor::auth::UserPermissions;
use crate::proto::reading_server::{Reading, ReadingServer};
use crate::proto::{
    Empty, Id, MangaReply, ReadingPatchRequest, ReadingPostRequest, UpdateChapterOffsetRequest,
};

#[derive(Debug, Default)]
pub struct MyReading {}

#[tonic::async_trait]
impl Reading for MyReading {
    /// Edit reading progress
    async fn update(
        &self,
        request: Request<ReadingPatchRequest>,
    ) -> Result<Response<MangaReply>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let logged_in = request.extensions().get::<entity::user::Model>().unwrap();
        let req = request.get_ref();

        let mut reading = entity::reading::Entity::find_by_id((logged_in.id, req.manga_id))
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
            get_manga_by_id(db, Some(logged_in), reading.manga_id).await?,
        ))
    }

    /// Add a new manga to reading
    async fn create(
        &self,
        request: Request<ReadingPostRequest>,
    ) -> Result<Response<MangaReply>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let logged_in = request.extensions().get::<entity::user::Model>().unwrap();
        let req = request.get_ref();

        let saved = entity::reading::ActiveModel {
            manga_id: Set(req.manga_id),
            user_id: Set(logged_in.id),
            ..Default::default()
        }
        .insert(db)
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

        let reading = get_manga_by_id(db, Some(logged_in), saved.manga_id).await?;

        Ok(Response::new(reading))
    }

    /// Delete a reading index
    async fn delete(&self, request: Request<Id>) -> Result<Response<Empty>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let logged_in = request.extensions().get::<entity::user::Model>().unwrap();
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

    /// Update the initial scroll offset of a chapter
    async fn update_chapter_offset(
        &self,
        request: Request<UpdateChapterOffsetRequest>,
    ) -> Result<Response<Empty>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let logged_in = request.extensions().get::<entity::user::Model>().unwrap();
        let req = request.get_ref();

        // Find offset or create new
        let model = entity::chapter_offset::Entity::find_by_id((logged_in.id, req.chapter_id))
            .one(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        // Save offset
        if let Some(model) = model {
            let mut model = model.into_active_model();
            model.offset = ActiveValue::Set(req.pixels);
            model.page = ActiveValue::Set(req.page);

            model
                .update(db)
                .await
                .map_err(|e| Status::internal(e.to_string()))?;
        } else {
            entity::chapter_offset::ActiveModel {
                user_id: ActiveValue::Set(logged_in.id),
                chapter_id: ActiveValue::Set(req.chapter_id),
                offset: ActiveValue::Set(req.pixels),
                page: ActiveValue::Set(req.page),
                ..Default::default()
            }
            .insert(db)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        }

        Ok(Response::new(Empty::default()))
    }
}

crate::export_service!(ReadingServer, MyReading, auth = UserPermissions::USER);
