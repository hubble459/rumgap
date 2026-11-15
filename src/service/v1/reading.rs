use migration::{Expr, JoinType, OnConflict};
use sea_orm::ActiveValue::{self, Set};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, RelationTrait,
};
use tonic::{Request, Response, Status};

use super::manga::get_manga_by_id;
use crate::interceptor::auth::UserPermissions;
use crate::proto::reading_server::{Reading, ReadingServer};
use crate::proto::reading_patch_request::Position;
use crate::proto::{
    CrossSourceOffsetReply, DeleteReadingRequest, Empty, GetCrossSourceOffsetRequest, MangaReply, ReadingPatchRequest,
    ReadingPostRequest, UpdateChapterOffsetRequest,
};
use crate::util::auth::Authorize;
use crate::util::db::DatabaseRequest;
use crate::util::{anilist_sync, read_event};

fn internal<E: ToString>(e: E) -> Status {
    Status::internal(e.to_string())
}

/// The one shared place `reading.progress` and `reading.last_canonical_chapter_id` are ever
/// written together, so the two can never drift apart across call sites (Phase 1c).
/// `progress` is redefined as "cached rank of `last_canonical_chapter_id`" - the count of
/// canonical chapters at or before this one for the manga - which keeps it working
/// unchanged as a plain int count for the common single-source case.
pub async fn sync_reading_progress<C: ConnectionTrait>(
    db: &C,
    user_id: i32,
    manga_id: i32,
    canonical_chapter_id: i32,
) -> Result<(), Status> {
    let canonical = entity::canonical_chapter::Entity::find_by_id(canonical_chapter_id)
        .filter(entity::canonical_chapter::Column::MangaId.eq(manga_id))
        .one(db)
        .await
        .map_err(internal)?
        .ok_or(Status::not_found("Canonical chapter not found for this manga"))?;

    let rank = entity::canonical_chapter::Entity::find()
        .filter(entity::canonical_chapter::Column::MangaId.eq(manga_id))
        .filter(entity::canonical_chapter::Column::Ordinal.lte(canonical.ordinal))
        .count(db)
        .await
        .map_err(internal)?;

    entity::reading::Entity::update_many()
        .col_expr(entity::reading::Column::Progress, Expr::value(rank as i32))
        .col_expr(
            entity::reading::Column::LastCanonicalChapterId,
            Expr::value(canonical_chapter_id),
        )
        .filter(entity::reading::Column::UserId.eq(user_id))
        .filter(entity::reading::Column::MangaId.eq(manga_id))
        .exec(db)
        .await
        .map_err(internal)?;

    Ok(())
}

#[derive(Debug, Default)]
pub struct ReadingController;

#[tonic::async_trait]
impl Reading for ReadingController {
    /// Edit reading progress
    async fn update(&self, request: Request<ReadingPatchRequest>) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?;
        let req = request.get_ref();

        let reading = entity::reading::Entity::find_by_id((logged_in.id, req.manga_id))
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("Reading not found"))?;

        // Older clients leave a progress of 0 off the wire entirely (proto3 default).
        let mut chapter_id = None;
        match req.position.unwrap_or(Position::Progress(0)) {
            Position::CanonicalChapterId(canonical_chapter_id) => {
                sync_reading_progress(db, logged_in.id, req.manga_id, canonical_chapter_id).await?;
            }
            #[allow(deprecated)]
            Position::ChapterId(id) => {
                let chapter = entity::chapter::Entity::find_by_id(id)
                    .one(db)
                    .await
                    .map_err(internal)?
                    .ok_or(Status::not_found("Chapter not found"))?;
                let canonical_chapter_id = chapter.canonical_chapter_id.ok_or(Status::failed_precondition(
                    "This chapter is not linked to a canonical chapter",
                ))?;
                sync_reading_progress(db, logged_in.id, req.manga_id, canonical_chapter_id).await?;
                chapter_id = Some(id);
            }
            // A raw rank leaves last_canonical_chapter_id as it is: there's no chapter to
            // derive it from.
            Position::Progress(progress) => {
                let mut reading = reading.into_active_model();
                reading.progress = Set(progress);
                reading.update(db).await.map_err(internal)?;
            }
        }
        read_event::record_progress(db, logged_in.id, req.manga_id, chapter_id).await?;
        anilist_sync::schedule(db.clone(), logged_in, req.manga_id);

        Ok(Response::new(get_manga_by_id(db, Some(logged_in), req.manga_id).await?))
    }

    /// Add a new manga to reading
    async fn create(&self, request: Request<ReadingPostRequest>) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?;
        let req = request.get_ref();

        entity::manga::Entity::find_by_id(req.manga_id)
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("Manga not found"))?;

        // Adding a manga that's already in the reading list just returns it.
        entity::reading::Entity::insert(entity::reading::ActiveModel {
            manga_id: Set(req.manga_id),
            user_id: Set(logged_in.id),
            ..Default::default()
        })
        .on_conflict(
            OnConflict::columns([entity::reading::Column::UserId, entity::reading::Column::MangaId])
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(db)
        .await
        .map_err(internal)?;

        let reading = get_manga_by_id(db, Some(logged_in), req.manga_id).await?;

        Ok(Response::new(reading))
    }

    /// Delete a reading index
    async fn delete(&self, request: Request<DeleteReadingRequest>) -> Result<Response<Empty>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?;
        let req = request.get_ref();

        // Delete reading
        let reading = entity::reading::Entity::delete_by_id((logged_in.id, req.manga_id))
            .exec(db)
            .await
            .map_err(internal)?;

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
        let db = request.db()?;
        let logged_in = request.authorize()?;
        let req = request.get_ref();

        // Find offset or create new
        let model = entity::chapter_offset::Entity::find_by_id((logged_in.id, req.chapter_id))
            .one(db)
            .await
            .map_err(internal)?;

        read_event::record_open(db, logged_in.id, req.chapter_id, model.as_ref().map(|m| m.updated_at)).await?;

        // Save offset
        if let Some(model) = model {
            let mut model = model.into_active_model();
            model.offset = ActiveValue::Set(req.pixels);
            model.page = ActiveValue::Set(req.page);
            model.fraction = ActiveValue::Set(req.fraction);

            model.update(db).await.map_err(internal)?;
        } else {
            entity::chapter_offset::ActiveModel {
                user_id: ActiveValue::Set(logged_in.id),
                chapter_id: ActiveValue::Set(req.chapter_id),
                offset: ActiveValue::Set(req.pixels),
                page: ActiveValue::Set(req.page),
                fraction: ActiveValue::Set(req.fraction),
                ..Default::default()
            }
            .insert(db)
            .await
            .map_err(internal)?;
        }

        Ok(Response::new(Empty::default()))
    }

    /// Cross-source scroll resume: the most recent `chapter_offset.fraction` recorded across
    /// any chapter sharing this canonical_chapter_id, for approximating "roughly the right
    /// spot" on a different source's version of the same chapter.
    async fn get_cross_source_offset(
        &self,
        request: Request<GetCrossSourceOffsetRequest>,
    ) -> Result<Response<CrossSourceOffsetReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?;
        let req = request.get_ref();

        let fraction: Option<Option<f32>> = entity::chapter_offset::Entity::find()
            .filter(entity::chapter_offset::Column::UserId.eq(logged_in.id))
            .filter(entity::chapter_offset::Column::Fraction.is_not_null())
            .join(JoinType::InnerJoin, entity::chapter_offset::Relation::Chapter.def())
            .filter(entity::chapter::Column::CanonicalChapterId.eq(req.canonical_chapter_id))
            .order_by_desc(entity::chapter_offset::Column::UpdatedAt)
            .select_only()
            .column(entity::chapter_offset::Column::Fraction)
            .into_tuple()
            .one(db)
            .await
            .map_err(internal)?;

        Ok(Response::new(CrossSourceOffsetReply {
            fraction: fraction.flatten(),
        }))
    }
}

crate::export_service!(ReadingServer, ReadingController, auth = UserPermissions::USER);

#[cfg(test)]
mod tests {
    use prost::Message;

    use super::*;

    #[test]
    #[allow(deprecated)]
    fn older_clients_still_decode() {
        // An older client sends `progress` (field 2) and then `chapter_id` (field 3); the
        // last oneof member on the wire wins, so the chapter is used, as it always was.
        let both = [0x08, 0x01, 0x10, 0x05, 0x18, 0x07];
        let request = ReadingPatchRequest::decode(&both[..]).unwrap();
        assert_eq!(request.position, Some(Position::ChapterId(7)));

        let progress_only = [0x08, 0x01, 0x10, 0x05];
        let request = ReadingPatchRequest::decode(&progress_only[..]).unwrap();
        assert_eq!(request.position, Some(Position::Progress(5)));

        let progress_zero = [0x08, 0x01];
        let request = ReadingPatchRequest::decode(&progress_zero[..]).unwrap();
        assert_eq!(request.position, None);
    }
}
