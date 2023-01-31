use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, DatabaseConnection, EntityTrait, IntoActiveModel,
};
use tonic::{Request, Response, Status};

use crate::interceptor::auth::{LoggedInUser, UserPermissions};
use crate::proto::reading_server::{Reading, ReadingServer};
use crate::proto::{
    Empty, Id, ReadingPatchRequest, ReadingPostRequest,
    MangaReply,
};

use super::manga::get_manga_by_id;

#[derive(Debug, Default)]
pub struct MyReading {}

#[tonic::async_trait]
impl Reading for MyReading {
    /// Edit reading progress
    async fn edit(
        &self,
        request: Request<ReadingPatchRequest>,
    ) -> Result<Response<MangaReply>, Status> {
        let db = request.extensions().get::<DatabaseConnection>().unwrap();
        let logged_in = request.extensions().get::<LoggedInUser>().unwrap();
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

        let reading = get_manga_by_id(db, Some(logged_in), saved.manga_id).await?;

        Ok(Response::new(reading))
    }

    /// Delete a reading index
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

crate::export_service!(ReadingServer, MyReading, auth = UserPermissions::USER);
