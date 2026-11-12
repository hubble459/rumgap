//! The append-only `read_event` log behind reading activity stats.

use chrono::{Duration, NaiveDateTime, Utc};
use sea_orm::{ConnectionTrait, Statement};
use tonic::Status;

/// Reopening a chapter after this long counts as reading it again.
const REOPEN_AFTER_MINUTES: i64 = 30;

fn internal<E: ToString>(e: E) -> Status {
    Status::internal(e.to_string())
}

/// Logs that `user_id` opened `chapter_id`, unless they were already reading it within the last
/// half hour (`last_seen_at` is when its scroll offset was last saved, `None` on a first open).
pub async fn record_open<C: ConnectionTrait>(
    db: &C,
    user_id: i32,
    chapter_id: i32,
    last_seen_at: Option<NaiveDateTime>,
) -> Result<(), Status> {
    if last_seen_at.is_some_and(|at| Utc::now().naive_utc() - at < Duration::minutes(REOPEN_AFTER_MINUTES)) {
        return Ok(());
    }
    db.execute_raw(Statement::from_sql_and_values(
        db.get_database_backend(),
        r#"
        INSERT INTO read_event (user_id, manga_id, chapter_id, kind)
        SELECT $1, s.manga_id, c.id, 'open'
        FROM chapter c
        JOIN manga_source s ON s.id = c.manga_source_id
        WHERE c.id = $2
        "#,
        [user_id.into(), chapter_id.into()],
    ))
    .await
    .map(|_| ())
    .map_err(internal)
}

/// Logs `user_id`'s reading progress on `manga_id` as it is now.
pub async fn record_progress<C: ConnectionTrait>(
    db: &C,
    user_id: i32,
    manga_id: i32,
    chapter_id: Option<i32>,
) -> Result<(), Status> {
    db.execute_raw(Statement::from_sql_and_values(
        db.get_database_backend(),
        r#"
        INSERT INTO read_event (user_id, manga_id, chapter_id, kind, progress)
        SELECT user_id, manga_id, $3, 'progress', progress
        FROM reading
        WHERE user_id = $1 AND manga_id = $2
        "#,
        [user_id.into(), manga_id.into(), chapter_id.into()],
    ))
    .await
    .map(|_| ())
    .map_err(internal)
}
