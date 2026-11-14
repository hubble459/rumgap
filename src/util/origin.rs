//! Where a manga comes from: manga (Japan), manhwa (Korea), manhua (China) or unknown.
//! Detected from its sources: the original language a site reports (MangaDex), else an
//! origin genre tag, else the script of its titles. A mod can set it by hand
//! (`manga.origin_set_by_hand`); detection then leaves it alone.

use log::{info, warn};
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use tonic::Status;

pub const MANGA: &str = "manga";
pub const MANHWA: &str = "manhwa";
pub const MANHUA: &str = "manhua";
pub const UNKNOWN: &str = "unknown";

/// What one manga's sources say about it.
#[derive(Default)]
pub struct Signals {
    pub languages: Vec<String>,
    pub genres: Vec<String>,
    pub titles: Vec<String>,
}

fn from_language(language: &str) -> Option<&'static str> {
    match language.to_lowercase().split('-').next()? {
        "ko" => Some(MANHWA),
        "ja" => Some(MANGA),
        "zh" => Some(MANHUA),
        _ => None,
    }
}

/// The one origin among `values`, or `None` when there's none or they disagree.
fn single(values: impl Iterator<Item = &'static str>) -> Option<&'static str> {
    let mut found = None;
    for value in values {
        match found {
            None => found = Some(value),
            Some(existing) if existing != value => return None,
            _ => {}
        }
    }
    found
}

fn from_scripts(titles: &[String]) -> Option<&'static str> {
    let chars = || titles.iter().flat_map(|title| title.chars());
    // Korean works often carry a Chinese alt title, and Japanese ones kanji, so the
    // script only Korean or Japanese uses wins over Chinese characters.
    if chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c) || ('\u{1100}'..='\u{11FF}').contains(&c)) {
        Some(MANHWA)
    } else if chars().any(|c| ('\u{3040}'..='\u{30FF}').contains(&c)) {
        Some(MANGA)
    } else if chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c)) {
        Some(MANHUA)
    } else {
        None
    }
}

pub fn detect(signals: &Signals) -> &'static str {
    single(signals.languages.iter().filter_map(|language| from_language(language)))
        .or_else(|| {
            single(
                signals
                    .genres
                    .iter()
                    .filter_map(|genre| match genre.trim().to_lowercase().as_str() {
                        "manhwa" => Some(MANHWA),
                        "manhua" => Some(MANHUA),
                        "manga" => Some(MANGA),
                        _ => None,
                    }),
            )
        })
        .or_else(|| from_scripts(&signals.titles))
        .unwrap_or(UNKNOWN)
}

const SIGNALS_QUERY: &str = r#"
    SELECT s.manga_id,
           array_remove(array_agg(DISTINCT s.original_language), NULL),
           coalesce((SELECT array_agg(g) FROM manga_source s2, unnest(s2.genres) g WHERE s2.manga_id = s.manga_id), '{}'),
           array_agg(s.title) || coalesce((SELECT array_agg(t) FROM manga_source s3, unnest(s3.alt_titles) t WHERE s3.manga_id = s.manga_id), '{}')
    FROM manga_source s
    JOIN manga m ON m.id = s.manga_id
    WHERE NOT m.origin_set_by_hand AND ($1::int IS NULL OR s.manga_id = $1)
    GROUP BY s.manga_id
"#;

/// Re-detects the origin of `manga_id`, or of every manga when `None`, skipping manga whose
/// origin was set by hand. Returns how many changed.
pub async fn update(db: &DatabaseConnection, manga_id: Option<i32>) -> Result<usize, Status> {
    let backend = db.get_database_backend();
    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            backend,
            SIGNALS_QUERY,
            [manga_id.into()],
        ))
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

    let mut changed = 0;
    for row in rows {
        let (Ok(id), Ok(languages), Ok(genres), Ok(titles)) = (
            row.try_get_by_index::<i32>(0),
            row.try_get_by_index::<Vec<String>>(1),
            row.try_get_by_index::<Vec<String>>(2),
            row.try_get_by_index::<Vec<String>>(3),
        ) else {
            continue;
        };
        let origin = detect(&Signals {
            languages,
            genres,
            titles,
        });
        let result = db
            .execute_raw(Statement::from_sql_and_values(
                backend,
                "UPDATE manga SET origin = $1 WHERE id = $2 AND origin <> $1 AND NOT origin_set_by_hand",
                [origin.into(), id.into()],
            ))
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        changed += result.rows_affected() as usize;
    }
    Ok(changed)
}

/// Startup pass over every manga, so rule changes reach existing data without a migration.
pub async fn update_all(db: &DatabaseConnection) {
    match update(db, None).await {
        Ok(0) => {}
        Ok(changed) => info!("[Origin] Updated the origin of {changed} manga"),
        Err(e) => warn!("[Origin] Couldn't update origins: {}", e.message()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signals(languages: &[&str], genres: &[&str], titles: &[&str]) -> Signals {
        let owned = |values: &[&str]| values.iter().map(|value| value.to_string()).collect();
        Signals {
            languages: owned(languages),
            genres: owned(genres),
            titles: owned(titles),
        }
    }

    #[test]
    fn language_wins() {
        assert_eq!(detect(&signals(&["ko"], &["Manga"], &["進撃の巨人"])), MANHWA);
        assert_eq!(detect(&signals(&["ja-ro"], &[], &[])), MANGA);
        assert_eq!(detect(&signals(&["zh-hk"], &[], &[])), MANHUA);
    }

    #[test]
    fn then_genre_tags() {
        assert_eq!(
            detect(&signals(&["en"], &["Action", "Manhwa"], &["나 혼자만 레벨업"])),
            MANHWA
        );
        assert_eq!(detect(&signals(&[], &["manhua"], &["Solo Leveling"])), MANHUA);
    }

    #[test]
    fn then_title_scripts() {
        assert_eq!(
            detect(&signals(&[], &[], &["Solo Leveling", "나 혼자만 레벨업", "我独自升级"])),
            MANHWA
        );
        assert_eq!(detect(&signals(&[], &[], &["進撃の巨人"])), MANGA);
        assert_eq!(detect(&signals(&[], &[], &["斗破苍穹"])), MANHUA);
    }

    #[test]
    fn disagreement_falls_through() {
        assert_eq!(detect(&signals(&["ko", "ja"], &["Manhua"], &[])), MANHUA);
        assert_eq!(detect(&signals(&[], &["Manhwa", "Manga"], &["Tower"])), UNKNOWN);
    }

    #[test]
    fn unknown_without_signals() {
        assert_eq!(detect(&signals(&[], &["Action"], &["Solo Leveling"])), UNKNOWN);
    }
}
