//! Pushes reading progress to AniList. Progress saved in quick succession (reading a few
//! chapters in a row, an offline batch) is debounced into one push per user and manga.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};

use crate::util::anilist::{self, AnilistError, ListEntry};

const DEBOUNCE: Duration = Duration::from_secs(30);

lazy_static! {
    static ref PENDING: Mutex<HashMap<(i32, i32), u64>> = Mutex::new(HashMap::new());
}
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Pushes the user's progress on the manga to AniList once it stops changing for a bit.
pub fn schedule(db: DatabaseConnection, user: &entity::user::Model, manga_id: i32) {
    if anilist::usable_token(user).is_none() {
        return;
    }
    let key = (user.id, manga_id);
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed);
    PENDING.lock().unwrap().insert(key, generation);

    tokio::spawn(async move {
        tokio::time::sleep(DEBOUNCE).await;
        {
            let mut pending = PENDING.lock().unwrap();
            if pending.get(&key) != Some(&generation) {
                return;
            }
            pending.remove(&key);
        }
        if let Err(e) = push(&db, key.0, key.1).await {
            warn!("[AniList] Failed to push progress of user {} on manga {}: {e}", key.0, key.1);
        }
    });
}

/// What to save on AniList, if anything: progress only goes up, and a finished manga is
/// marked completed.
pub fn decide(current: Option<&ListEntry>, progress: i32, completed: bool) -> Option<ListEntry> {
    if progress <= 0 {
        return None;
    }
    match current {
        Some(entry) if entry.progress > progress => None,
        Some(entry) if entry.progress == progress => {
            let finish = completed && !matches!(entry.status.as_str(), "COMPLETED" | "REPEATING");
            finish.then(|| ListEntry {
                progress,
                status: "COMPLETED".to_string(),
            })
        }
        _ => {
            let status = if completed {
                "COMPLETED"
            } else if current.is_some_and(|e| e.status == "REPEATING") {
                "REPEATING"
            } else {
                "CURRENT"
            };
            Some(ListEntry {
                progress,
                status: status.to_string(),
            })
        }
    }
}

async fn push(db: &DatabaseConnection, user_id: i32, manga_id: i32) -> Result<(), AnilistError> {
    let db_err = |e: sea_orm::DbErr| AnilistError::Other(e.to_string());
    let Some(user) = anilist::find_user(db, user_id).await.map_err(db_err)? else {
        return Ok(());
    };
    let Some(token) = anilist::usable_token(&user) else {
        return Ok(());
    };
    let Some(manga) = entity::manga::Entity::find_by_id(manga_id).one(db).await.map_err(db_err)? else {
        return Ok(());
    };
    let Some(anilist_id) = manga.anilist_id else {
        return Ok(());
    };
    let Some(reading) = entity::reading::Entity::find_by_id((user_id, manga_id))
        .one(db)
        .await
        .map_err(db_err)?
    else {
        return Ok(());
    };

    let last_canonical = match reading.last_canonical_chapter_id {
        Some(id) => entity::canonical_chapter::Entity::find_by_id(id).one(db).await.map_err(db_err)?,
        None => None,
    };
    let progress = match &last_canonical {
        Some(canonical) => canonical.ordinal.trunc().try_into().unwrap_or(reading.progress),
        None => reading.progress,
    };

    let primary = entity::manga_source::Entity::find()
        .filter(entity::manga_source::Column::MangaId.eq(manga_id))
        .order_by_desc(entity::manga_source::Column::IsPrimary)
        .one(db)
        .await
        .map_err(db_err)?;
    let newest = entity::canonical_chapter::Entity::find()
        .filter(entity::canonical_chapter::Column::MangaId.eq(manga_id))
        .order_by_desc(entity::canonical_chapter::Column::Ordinal)
        .one(db)
        .await
        .map_err(db_err)?;
    let finished = primary.is_some_and(|s| !s.is_ongoing);
    let read_newest = match (&last_canonical, &newest) {
        (Some(last), Some(newest)) => last.id == newest.id,
        _ => false,
    };

    let result = async {
        let current = anilist::list_entry(token, anilist_id).await?;
        let Some(entry) = decide(current.as_ref(), progress, finished && read_newest) else {
            return Ok(());
        };
        anilist::save_progress(token, anilist_id, &entry).await?;
        debug!(
            "[AniList] User {user_id}: AniList {anilist_id} at {} ({})",
            entry.progress, entry.status
        );
        Ok(())
    }
    .await;

    if let Err(AnilistError::Unauthorized) = result {
        info!("[AniList] Token of user {user_id} was rejected, disconnecting");
        anilist::disconnect(db, user_id).await.map_err(db_err)?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(progress: i32, status: &str) -> ListEntry {
        ListEntry {
            progress,
            status: status.to_string(),
        }
    }

    #[test]
    fn never_lowers_progress() {
        assert_eq!(decide(Some(&entry(50, "CURRENT")), 40, false), None);
        assert_eq!(decide(Some(&entry(50, "CURRENT")), 40, true), None);
    }

    #[test]
    fn same_progress_only_marks_completed() {
        assert_eq!(decide(Some(&entry(40, "CURRENT")), 40, false), None);
        assert_eq!(decide(Some(&entry(40, "CURRENT")), 40, true), Some(entry(40, "COMPLETED")));
        assert_eq!(decide(Some(&entry(40, "COMPLETED")), 40, true), None);
    }

    #[test]
    fn higher_progress_is_saved() {
        assert_eq!(decide(None, 3, false), Some(entry(3, "CURRENT")));
        assert_eq!(decide(Some(&entry(2, "PLANNING")), 3, false), Some(entry(3, "CURRENT")));
        assert_eq!(decide(Some(&entry(2, "DROPPED")), 3, false), Some(entry(3, "CURRENT")));
        assert_eq!(decide(Some(&entry(2, "REPEATING")), 3, false), Some(entry(3, "REPEATING")));
        assert_eq!(decide(Some(&entry(2, "CURRENT")), 10, true), Some(entry(10, "COMPLETED")));
    }

    #[test]
    fn nothing_read_is_not_pushed() {
        assert_eq!(decide(None, 0, false), None);
    }
}
