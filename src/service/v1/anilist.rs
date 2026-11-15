use chrono::Utc;
use entity::manga::AnilistMatch;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait};
use tonic::{Request, Response, Status};

use super::manga::get_manga_by_id;
use crate::interceptor::auth::UserPermissions;
use crate::proto::anilist_server::{Anilist, AnilistServer};
use crate::proto::{
    AnilistConnectUrlReply, AnilistConnectUrlRequest, AnilistReturnTarget, AnilistSearchReply, AnilistSearchRequest,
    AnilistStatusReply, Empty, MangaReply, SetAnilistMatchRequest,
};
use crate::util::anilist::{self, ReturnTarget};
use crate::util::anilist_match;
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
        }
        info!(
            "[AniList] User {} set manga {} AniList match to {:?} (rematch: {})",
            logged_in.id, req.manga_id, req.anilist_id, req.rematch
        );

        Ok(Response::new(get_manga_by_id(db, Some(logged_in), req.manga_id).await?))
    }
}

crate::export_service!(AnilistServer, AnilistController, auth = UserPermissions::USER);
