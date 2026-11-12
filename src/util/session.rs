//! Login sessions backing the bearer tokens (see `interceptor::auth`). A token names its
//! `session` row, and is only accepted while that row exists and hasn't expired, so
//! deleting rows is how one session, or every session of a user, gets revoked.
//!
//! Expiry slides: every use pushes `expires_at` to `SESSION_TTL_DAYS` from now, so a
//! device that keeps getting used stays logged in, independently of the user's other
//! devices.

use chrono::{Duration, NaiveDateTime, Utc};
use rand::RngExt;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use tonic::Status;

use crate::interceptor::auth::sign;

/// How often `last_used_at`/`expires_at` get written back, so not every request costs an
/// UPDATE.
const TOUCH_INTERVAL_MINUTES: i64 = 60;

fn ttl() -> Duration {
    Duration::days(
        std::env::var("SESSION_TTL_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(90),
    )
}

fn internal(e: impl ToString) -> Status {
    Status::internal(e.to_string())
}

/// Start a new session for `user_id` and return its signed bearer token.
pub async fn create(db: &DatabaseConnection, user_id: i32, user_agent: Option<&str>) -> Result<String, Status> {
    let id = format!("{:032x}", rand::rng().random::<u128>());
    let now = Utc::now().naive_utc();

    entity::session::ActiveModel {
        id: Set(id.clone()),
        user_id: Set(user_id),
        user_agent: Set(user_agent.map(|ua| ua.chars().take(512).collect())),
        created_at: Set(now),
        last_used_at: Set(now),
        expires_at: Set(now + ttl()),
    }
    .insert(db)
    .await
    .map_err(internal)?;

    sign(user_id, &id).map_err(internal)
}

/// The session and its user, if `session_id` belongs to `user_id` and hasn't expired.
/// Slides the expiry forward when it was last touched more than `TOUCH_INTERVAL_MINUTES` ago.
pub async fn validate(
    db: &DatabaseConnection,
    user_id: i32,
    session_id: &str,
) -> Result<Option<(entity::session::Model, entity::user::Model)>, Status> {
    let Some((session, Some(user))) = entity::session::Entity::find_by_id(session_id)
        .find_also_related(entity::user::Entity)
        .one(db)
        .await
        .map_err(internal)?
    else {
        return Ok(None);
    };

    let now = Utc::now().naive_utc();
    if session.user_id != user_id || session.expires_at <= now {
        return Ok(None);
    }

    let session = if now - session.last_used_at > Duration::minutes(TOUCH_INTERVAL_MINUTES) {
        touch(db, session, now).await?
    } else {
        session
    };

    Ok(Some((session, user)))
}

async fn touch(
    db: &DatabaseConnection,
    session: entity::session::Model,
    now: NaiveDateTime,
) -> Result<entity::session::Model, Status> {
    let mut active: entity::session::ActiveModel = session.into();
    active.last_used_at = Set(now);
    active.expires_at = Set(now + ttl());
    active.update(db).await.map_err(internal)
}

/// Revoke one of `user_id`'s sessions. Returns whether it existed.
pub async fn revoke(db: &DatabaseConnection, user_id: i32, session_id: &str) -> Result<bool, Status> {
    let result = entity::session::Entity::delete_many()
        .filter(entity::session::Column::UserId.eq(user_id))
        .filter(entity::session::Column::Id.eq(session_id))
        .exec(db)
        .await
        .map_err(internal)?;
    Ok(result.rows_affected > 0)
}

/// Revoke every session of `user_id`, except `keep` when given.
pub async fn revoke_all(db: &DatabaseConnection, user_id: i32, keep: Option<&str>) -> Result<u64, Status> {
    let mut query = entity::session::Entity::delete_many().filter(entity::session::Column::UserId.eq(user_id));
    if let Some(keep) = keep {
        query = query.filter(entity::session::Column::Id.ne(keep));
    }
    Ok(query.exec(db).await.map_err(internal)?.rows_affected)
}

/// Delete expired sessions. Called periodically from the auto-update loop.
pub async fn prune(db: &DatabaseConnection) {
    let result = entity::session::Entity::delete_many()
        .filter(entity::session::Column::ExpiresAt.lte(Utc::now().naive_utc()))
        .exec(db)
        .await;
    match result {
        Ok(res) if res.rows_affected > 0 => info!("Pruned {} expired session(s)", res.rows_affected),
        Err(e) => warn!("Failed to prune expired sessions: {}", e),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use sea_orm::Database;

    use super::*;
    use crate::interceptor::auth::Token;

    /// Requires DATABASE_URL to point at a throwaway, already-migrated test database (never
    /// the real one). Run with:
    ///   DATABASE_URL=postgres://... JWT_SECRET=test cargo test sessions -- --ignored
    #[ignore]
    #[tokio::test]
    async fn sessions() {
        use jwt::VerifyWithKey;

        let db_url = std::env::var("DATABASE_URL").expect("set DATABASE_URL to a throwaway test DB");
        let db = Database::connect(db_url).await.unwrap();

        let user = entity::user::ActiveModel {
            username: Set(format!("s{}", rand::rng().random::<u32>())),
            email: Set(format!("{}@test", rand::rng().random::<u32>())),
            password_hash: Set("x".into()),
            preferred_hostnames: Set(vec![]),
            device_ids: Set(vec![]),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();

        let phone = create(&db, user.id, Some("phone")).await.unwrap();
        let desktop = create(&db, user.id, None).await.unwrap();
        let laptop = create(&db, user.id, None).await.unwrap();
        let sid = |token: &str| -> String {
            let token: Token = token
                .verify_with_key(&crate::interceptor::auth::SECRET_KEY.clone())
                .unwrap();
            token.sid
        };
        let (phone, desktop, laptop) = (sid(&phone), sid(&desktop), sid(&laptop));
        assert_ne!(phone, desktop);

        let (session, found_user) = validate(&db, user.id, &phone).await.unwrap().unwrap();
        assert_eq!(found_user.id, user.id);
        assert_eq!(session.user_agent.as_deref(), Some("phone"));
        assert!(validate(&db, user.id + 1, &phone).await.unwrap().is_none(), "wrong user");

        assert!(revoke(&db, user.id, &phone).await.unwrap());
        assert!(validate(&db, user.id, &phone).await.unwrap().is_none(), "revoked");
        assert!(validate(&db, user.id, &desktop).await.unwrap().is_some(), "other devices unaffected");

        assert_eq!(revoke_all(&db, user.id, Some(&desktop)).await.unwrap(), 1);
        assert!(validate(&db, user.id, &laptop).await.unwrap().is_none());
        assert!(validate(&db, user.id, &desktop).await.unwrap().is_some(), "kept");

        let stale = Utc::now().naive_utc() - Duration::hours(2);
        entity::session::Entity::update_many()
            .col_expr(entity::session::Column::LastUsedAt, stale.into())
            .col_expr(entity::session::Column::ExpiresAt, (stale + Duration::days(1)).into())
            .filter(entity::session::Column::Id.eq(&desktop))
            .exec(&db)
            .await
            .unwrap();
        let (session, _) = validate(&db, user.id, &desktop).await.unwrap().unwrap();
        assert!(session.expires_at > Utc::now().naive_utc() + ttl() - Duration::minutes(1), "slid");

        entity::session::Entity::update_many()
            .col_expr(entity::session::Column::ExpiresAt, stale.into())
            .filter(entity::session::Column::Id.eq(&desktop))
            .exec(&db)
            .await
            .unwrap();
        assert!(validate(&db, user.id, &desktop).await.unwrap().is_none(), "expired");
        prune(&db).await;
        assert!(entity::session::Entity::find_by_id(&desktop).one(&db).await.unwrap().is_none(), "pruned");

        entity::user::Entity::delete_by_id(user.id).exec(&db).await.unwrap();
    }
}
