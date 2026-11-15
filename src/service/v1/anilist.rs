use chrono::Utc;
use entity::manga::AnilistMatch;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait};
use tonic::{Request, Response, Status};

use super::manga::get_manga_by_id;
use crate::interceptor::auth::UserPermissions;
use crate::proto::anilist_server::{Anilist, AnilistServer};
use crate::proto::{
    AnilistConnectUrlReply, AnilistConnectUrlRequest, AnilistListEntryReply, AnilistListEntryRequest,
    AnilistListStatus, AnilistReturnTarget, AnilistSearchReply, AnilistSearchRequest, AnilistStatusReply, Empty,
    MangaReply, SetAnilistListStatusRequest, SetAnilistMatchRequest,
};
use crate::util::anilist::{self, ListEntry, ReturnTarget};
use crate::util::{anilist_match, anilist_sync};
use crate::util::auth::Authorize;
use crate::util::db::DatabaseRequest;

fn internal<E: ToString>(e: E) -> Status {
    Status::internal(e.to_string())
}

#[derive(Debug, Default)]
pub struct AnilistController;

#[tonic::async_trait]
impl Anilist for AnilistController {
    async fn connect_url(
        &self,
        request: Request<AnilistConnectUrlRequest>,
    ) -> Result<Response<AnilistConnectUrlReply>, Status> {
        let logged_in = request.authorize()?;
        let target = match request.get_ref().return_target() {
            AnilistReturnTarget::AnilistReturnWeb => ReturnTarget::Web,
            AnilistReturnTarget::AnilistReturnApp => ReturnTarget::App,
        };
        let url = anilist::authorize_url(&anilist::sign_state(logged_in.id, target))?;
        Ok(Response::new(AnilistConnectUrlReply { url }))
    }

    async fn status(&self, request: Request<Empty>) -> Result<Response<AnilistStatusReply>, Status> {
        let logged_in = request.authorize()?;
        let expires_at = logged_in.anilist_token_expires_at.map(|at| at.and_utc());
        let reply = AnilistStatusReply {
            connected: anilist::usable_token(logged_in).is_some(),
            user_id: logged_in.anilist_user_id,
            user_name: logged_in.anilist_user_name.clone(),
            expires_at: expires_at.map(|at| at.timestamp_millis()),
            expires_soon: expires_at
                .is_some_and(|at| at - Utc::now() < chrono::Duration::days(anilist::EXPIRY_WARN_DAYS)),
        };
        Ok(Response::new(reply))
    }

    async fn disconnect(&self, request: Request<Empty>) -> Result<Response<Empty>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?;
        anilist::disconnect(db, logged_in.id).await.map_err(internal)?;
        info!("[AniList] User {} disconnected AniList", logged_in.id);
        Ok(Response::new(Empty::default()))
    }

    async fn search(&self, request: Request<AnilistSearchRequest>) -> Result<Response<AnilistSearchReply>, Status> {
        let query = request.get_ref().query.trim();
        if query.is_empty() {
            return Ok(Response::new(AnilistSearchReply::default()));
        }
        let items = anilist::search_manga(query)
            .await?
            .into_iter()
            .map(anilist::Media::into_proto)
            .collect();
        Ok(Response::new(AnilistSearchReply { items }))
    }

    async fn set_match(&self, request: Request<SetAnilistMatchRequest>) -> Result<Response<MangaReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?;
        let req = request.get_ref();

        entity::manga::Entity::find_by_id(req.manga_id)
            .one(db)
            .await
            .map_err(internal)?
            .ok_or(Status::not_found("Manga not found"))?;

        if req.rematch {
            entity::manga::ActiveModel {
                id: Set(req.manga_id),
                anilist_match: Set(None),
                ..Default::default()
            }
            .update(db)
            .await
            .map_err(internal)?;
            anilist_match::match_manga(db, req.manga_id).await?;
        } else {
            let media = match req.anilist_id {
                Some(id) => Some(
                    anilist::media(id)
                        .await?
                        .ok_or(Status::not_found("AniList entry not found"))?,
                ),
                None => None,
            };
            anilist::set_match(db, req.manga_id, media.as_ref(), AnilistMatch::Manual)
                .await
                .map_err(internal)?;
            if media.is_some() {
                anilist_sync::sync_readers(db.clone(), req.manga_id);
            }
        }
        info!(
            "[AniList] User {} set manga {} AniList match to {:?} (rematch: {})",
            logged_in.id, req.manga_id, req.anilist_id, req.rematch
        );

        Ok(Response::new(get_manga_by_id(db, Some(logged_in), req.manga_id).await?))
    }

    async fn list_entry(
        &self,
        request: Request<AnilistListEntryRequest>,
    ) -> Result<Response<AnilistListEntryReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?;
        let manga_id = request.get_ref().manga_id;
        let mut reply = AnilistListEntryReply {
            connected: anilist::usable_token(logged_in).is_some(),
            ..Default::default()
        };

        let Some((token, anilist_id)) = anilist_sync::target(db, logged_in.id, manga_id).await? else {
            reply.matched = manga_matched(db, manga_id).await?;
            return Ok(Response::new(reply));
        };
        reply.matched = true;
        let entry = anilist_sync::on_error(db, logged_in.id, anilist::list_entry(&token, anilist_id).await).await?;
        if let Some(entry) = entry {
            reply.status = status_to_proto(&entry.status).into();
            reply.progress = entry.progress;
        }
        Ok(Response::new(reply))
    }

    async fn set_list_status(
        &self,
        request: Request<SetAnilistListStatusRequest>,
    ) -> Result<Response<AnilistListEntryReply>, Status> {
        let db = request.db()?;
        let logged_in = request.authorize()?;
        let req = request.get_ref();
        let Some((token, anilist_id)) = anilist_sync::target(db, logged_in.id, req.manga_id).await? else {
            return Err(Status::failed_precondition(
                "Connect AniList and make sure this manga has an AniList match first",
            ));
        };

        let status = req.status();
        let result = async {
            let Some(status) = status_from_proto(status) else {
                anilist::delete_entry(&token, anilist_id).await?;
                return Ok(AnilistListEntryReply {
                    connected: true,
                    matched: true,
                    ..Default::default()
                });
            };
            // Keep whichever progress is further, so picking a status never loses any.
            let current = anilist::list_entry(&token, anilist_id).await?;
            let ours = anilist_sync::local_progress(db, logged_in.id, req.manga_id)
                .await?
                .map_or(0, |(progress, _)| progress);
            let entry = ListEntry {
                progress: ours.max(current.map_or(0, |e| e.progress)),
                status: status.to_string(),
            };
            anilist::save_progress(&token, anilist_id, &entry).await?;
            Ok(AnilistListEntryReply {
                connected: true,
                matched: true,
                status: status_to_proto(&entry.status).into(),
                progress: entry.progress,
            })
        }
        .await;
        let reply = anilist_sync::on_error(db, logged_in.id, result).await?;
        info!(
            "[AniList] User {} set the AniList status of manga {} to {:?}",
            logged_in.id, req.manga_id, status
        );
        Ok(Response::new(reply))
    }
}

async fn manga_matched(db: &sea_orm::DatabaseConnection, manga_id: i32) -> Result<bool, Status> {
    Ok(entity::manga::Entity::find_by_id(manga_id)
        .one(db)
        .await
        .map_err(internal)?
        .is_some_and(|m| m.anilist_id.is_some()))
}

fn status_to_proto(status: &str) -> AnilistListStatus {
    match status {
        "CURRENT" => AnilistListStatus::AnilistListCurrent,
        "PLANNING" => AnilistListStatus::AnilistListPlanning,
        "COMPLETED" => AnilistListStatus::AnilistListCompleted,
        "DROPPED" => AnilistListStatus::AnilistListDropped,
        "PAUSED" => AnilistListStatus::AnilistListPaused,
        "REPEATING" => AnilistListStatus::AnilistListRepeating,
        _ => AnilistListStatus::AnilistListNone,
    }
}

fn status_from_proto(status: AnilistListStatus) -> Option<&'static str> {
    match status {
        AnilistListStatus::AnilistListNone => None,
        AnilistListStatus::AnilistListCurrent => Some("CURRENT"),
        AnilistListStatus::AnilistListPlanning => Some("PLANNING"),
        AnilistListStatus::AnilistListCompleted => Some("COMPLETED"),
        AnilistListStatus::AnilistListDropped => Some("DROPPED"),
        AnilistListStatus::AnilistListPaused => Some("PAUSED"),
        AnilistListStatus::AnilistListRepeating => Some("REPEATING"),
    }
}

crate::export_service!(AnilistServer, AnilistController, auth = UserPermissions::USER);
