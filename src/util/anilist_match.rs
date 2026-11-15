//! Finds each manga's AniList entry by title, and keeps the cached AniList details fresh.
//! Like `title_match`, only exact matches after normalization count: a wrong match pushes
//! progress to the wrong AniList entry, while a missed one can be picked by hand.

use std::collections::HashSet;
use std::time::Duration;

use chrono::Utc;
use entity::manga::AnilistMatch;
use migration::Expr;
use sea_orm::{ColumnTrait, Condition, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::util::anilist::{self, AnilistError, Media};
use crate::util::title_match::normalize_title;

/// Titles searched per manga, at most.
const MAX_SEARCHES: usize = 3;
const REFRESH_AFTER_DAYS: i64 = 7;
/// A manga nothing was found for is searched again after this long, its sources may have
/// gained titles meanwhile.
const RETRY_NONE_AFTER_DAYS: i64 = 30;
const BATCH: u64 = 10;

/// The first of `results` that goes by one of `titles`.
pub fn pick<'a>(titles: &[String], results: &'a [Media]) -> Option<&'a Media> {
    let ours: HashSet<String> = titles
        .iter()
        .map(|t| normalize_title(t))
        .filter(|t| !t.is_empty())
        .collect();
    results
        .iter()
        .find(|media| media.titles().any(|t| ours.contains(&normalize_title(t))))
}

/// Every title of every source of the manga, the primary's title first.
async fn manga_titles(db: &DatabaseConnection, manga_id: i32) -> Result<Vec<String>, sea_orm::DbErr> {
    let sources = entity::manga_source::Entity::find()
        .filter(entity::manga_source::Column::MangaId.eq(manga_id))
        .order_by_desc(entity::manga_source::Column::IsPrimary)
        .order_by_asc(entity::manga_source::Column::Id)
        .all(db)
        .await?;
    let mut seen = HashSet::new();
    let titles = sources
        .iter()
        .map(|s| &s.title)
        .chain(sources.iter().flat_map(|s| s.alt_titles.iter()))
        .filter(|t| !t.trim().is_empty())
        .filter(|t| seen.insert(normalize_title(t)))
        .cloned()
        .collect();
    Ok(titles)
}

/// Searches AniList for the manga and stores what it finds. Leaves manual matches alone.
pub async fn match_manga(db: &DatabaseConnection, manga_id: i32) -> Result<Option<i32>, AnilistError> {
    let db_err = |e: sea_orm::DbErr| AnilistError::Other(e.to_string());
    let Some(manga) = entity::manga::Entity::find_by_id(manga_id).one(db).await.map_err(db_err)? else {
        return Ok(None);
    };
    if manga.anilist_match == Some(AnilistMatch::Manual) {
        return Ok(manga.anilist_id);
    }

    let titles = manga_titles(db, manga_id).await.map_err(db_err)?;
    let mut found = None;
    for query in titles.iter().take(MAX_SEARCHES) {
        let results = anilist::search_manga(query).await?;
        if let Some(media) = pick(&titles, &results) {
            found = Some(media.clone());
            break;
        }
    }

    match &found {
        Some(media) => {
            anilist::set_match(db, manga_id, Some(media), AnilistMatch::Auto).await.map_err(db_err)?;
            crate::util::anilist_sync::sync_readers(db.clone(), manga_id);
            debug!("[AniList] Matched manga {manga_id} to {}", media.id);
        }
        None => {
            anilist::set_match(db, manga_id, None, AnilistMatch::None).await.map_err(db_err)?;
            debug!("[AniList] No match for manga {manga_id}");
        }
    }
    Ok(found.map(|m| m.id))
}

/// Re-fetches the cached AniList details of a matched manga.
pub async fn refresh_manga(db: &DatabaseConnection, manga_id: i32, anilist_id: i32) -> Result<(), AnilistError> {
    let db_err = |e: sea_orm::DbErr| AnilistError::Other(e.to_string());
    match anilist::media(anilist_id).await? {
        Some(media) => anilist::store_media(db, manga_id, &media).await.map_err(db_err),
        // The entry was deleted from AniList; look for it again.
        None => match_manga(db, manga_id).await.map(|_| ()),
    }
}

/// Matches manga that were never matched (manga someone reads first) and refreshes stale
/// details, a few at a time. The AniList rate limit sets the pace.
pub async fn watch(db: DatabaseConnection) {
    if anilist::config().is_err() {
        info!("[AniList] Not configured, not matching manga");
        return;
    }
    loop {
        let did_work = match tick(&db).await {
            Ok(did_work) => did_work,
            Err(e) => {
                warn!("[AniList] Matching failed: {e}");
                false
            }
        };
        if !did_work {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    }
}

async fn tick(db: &DatabaseConnection) -> Result<bool, AnilistError> {
    use entity::manga::Column;
    let db_err = |e: sea_orm::DbErr| AnilistError::Other(e.to_string());
    let now = Utc::now().naive_utc();
    let read_first = Expr::cust("EXISTS (SELECT 1 FROM reading WHERE reading.manga_id = manga.id)");

    let unmatched: Vec<i32> = entity::manga::Entity::find()
        .filter(
            Condition::any().add(Column::AnilistMatch.is_null()).add(
                Condition::all()
                    .add(Column::AnilistMatch.eq(AnilistMatch::None))
                    .add(Column::AnilistMatchedAt.lt(now - chrono::Duration::days(RETRY_NONE_AFTER_DAYS))),
            ),
        )
        .select_only()
        .column(Column::Id)
        .order_by_desc(read_first.clone())
        .order_by_desc(Column::Id)
        .limit(BATCH)
        .into_tuple()
        .all(db)
        .await
        .map_err(db_err)?;
    for manga_id in &unmatched {
        if let Err(e) = match_manga(db, *manga_id).await {
            warn!("[AniList] Failed to match manga {manga_id}: {e}");
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }

    let stale: Vec<(i32, Option<i32>)> = entity::manga::Entity::find()
        .filter(Column::AnilistId.is_not_null())
        .filter(
            Condition::any()
                .add(Column::AnilistFetchedAt.is_null())
                .add(Column::AnilistFetchedAt.lt(now - chrono::Duration::days(REFRESH_AFTER_DAYS))),
        )
        .select_only()
        .column(Column::Id)
        .column(Column::AnilistId)
        .order_by_desc(read_first)
        .limit(BATCH)
        .into_tuple()
        .all(db)
        .await
        .map_err(db_err)?;
    for (manga_id, anilist_id) in &stale {
        let Some(anilist_id) = anilist_id else { continue };
        if let Err(e) = refresh_manga(db, *manga_id, *anilist_id).await {
            warn!("[AniList] Failed to refresh manga {manga_id}: {e}");
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }

    Ok(!unmatched.is_empty() || !stale.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::anilist::MediaTitle;

    fn media(id: i32, romaji: &str, english: Option<&str>, synonyms: &[&str]) -> Media {
        Media {
            id,
            title: MediaTitle {
                romaji: Some(romaji.to_string()),
                english: english.map(str::to_string),
                native: None,
            },
            synonyms: synonyms.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn titles(titles: &[&str]) -> Vec<String> {
        titles.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn matches_english_title_after_normalizing() {
        let results = [
            media(1, "Ore Dake Level Up na Ken", Some("Solo Leveling"), &[]),
            media(2, "Ore Dake Level Up na Ken: Ragnarok", Some("Solo Leveling: Ragnarok"), &[]),
        ];
        assert_eq!(pick(&titles(&["SOLO LEVELING!"]), &results).map(|m| m.id), Some(1));
    }

    #[test]
    fn matches_romaji_or_synonym_via_alt_titles() {
        let results = [media(7, "Shingeki no Kyojin", Some("Attack on Titan"), &["AoT", "SnK"])];
        assert_eq!(pick(&titles(&["Some Unknown Name", "Shingeki no Kyojin"]), &results).map(|m| m.id), Some(7));
        assert_eq!(pick(&titles(&["snk"]), &results).map(|m| m.id), Some(7));
    }

    #[test]
    fn rejects_near_misses() {
        let results = [media(1, "Ore Dake Level Up na Ken", Some("Solo Leveling"), &[])];
        assert!(pick(&titles(&["Solo Leveling 2"]), &results).is_none());
        assert!(pick(&titles(&["Solo"]), &results).is_none());
        assert!(pick(&titles(&[""]), &results).is_none());
    }
}
