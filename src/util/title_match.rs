//! Decides whether a freshly scraped manga is one the catalog already has, so it can be
//! added as a source instead of becoming a duplicate. Only exact matches after
//! normalization count: a wrong merge glues two different manga together, while a missed
//! one just leaves a duplicate that shows up under "Similar" and can be merged by hand.

use std::collections::{HashMap, HashSet};

use log::info;
use sea_orm::{ConnectionTrait, DatabaseConnection};
use tonic::Status;

/// When both sides have at least this many chapters, their counts must be within
/// `MAX_CHAPTER_RATIO` of each other to match.
const MIN_CHAPTERS_TO_COMPARE: i64 = 20;
const MAX_CHAPTER_RATIO: i64 = 4;

/// Lowercase, punctuation and symbols dropped, whitespace collapsed, a leading "the"
/// dropped. Digits stay, so "Solo Leveling" and "Solo Leveling 2" stay apart.
pub fn normalize_title(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    match words.as_slice() {
        ["the", rest @ ..] if !rest.is_empty() => rest.join(" "),
        _ => words.join(" "),
    }
}

fn normalized_set<'a>(titles: impl IntoIterator<Item = &'a String>) -> HashSet<String> {
    titles
        .into_iter()
        .map(|title| normalize_title(title))
        .filter(|title| !title.is_empty())
        .collect()
}

pub struct Candidate {
    pub manga_id: i32,
    pub titles: Vec<String>,
    pub chapters: i64,
}

fn chapter_counts_agree(a: i64, b: i64) -> bool {
    let (low, high) = (a.min(b), a.max(b));
    low < MIN_CHAPTERS_TO_COMPARE || high <= low * MAX_CHAPTER_RATIO
}

/// The one manga among `candidates` (one entry per source) sharing a normalized title with
/// `titles`, or `None` when there is none or more than one.
pub fn pick_match(titles: &[String], chapters: i64, candidates: &[Candidate]) -> Option<i32> {
    let wanted = normalized_set(titles);
    let mut matched: HashMap<i32, i64> = HashMap::new();
    for candidate in candidates {
        if !normalized_set(&candidate.titles).is_disjoint(&wanted) {
            let most = matched.entry(candidate.manga_id).or_default();
            *most = (*most).max(candidate.chapters);
        }
    }

    match matched.into_iter().collect::<Vec<_>>().as_slice() {
        [(manga_id, most)] if chapter_counts_agree(chapters, *most) => Some(*manga_id),
        _ => None,
    }
}

/// The existing manga `scraped` belongs to, if any. Candidates are narrowed with the
/// pg_trgm `%` operator first, then compared exactly in Rust.
pub async fn find_title_match(
    db: &DatabaseConnection,
    scraped: &manga_parser::model::Manga,
) -> Result<Option<i32>, Status> {
    let titles: Vec<String> = std::iter::once(&scraped.title)
        .chain(&scraped.alternative_titles)
        .filter(|title| !title.trim().is_empty())
        .cloned()
        .collect();
    if titles.is_empty() {
        return Ok(None);
    }

    let rows = db
        .query_all_raw(sea_orm::Statement::from_sql_and_values(
            db.get_database_backend(),
            r#"
            SELECT s.manga_id, s.title, s.alt_titles,
                   (SELECT count(*) FROM chapter c WHERE c.manga_source_id = s.id) AS chapters
            FROM manga_source s
            WHERE EXISTS (
                SELECT 1 FROM unnest($1::text[]) t WHERE t % any(s.alt_titles || s.title)
            )
            "#,
            [titles.clone().into()],
        ))
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

    let candidates = rows
        .iter()
        .map(|row| {
            let title: String = row.try_get_by_index(1)?;
            let alt_titles: Vec<String> = row.try_get_by_index(2)?;
            Ok(Candidate {
                manga_id: row.try_get_by_index(0)?,
                titles: std::iter::once(title).chain(alt_titles).collect(),
                chapters: row.try_get_by_index(3)?,
            })
        })
        .collect::<Result<Vec<_>, sea_orm::DbErr>>()
        .map_err(|e| Status::internal(e.to_string()))?;

    let matched = pick_match(&titles, scraped.chapters.len() as i64, &candidates);
    if let Some(manga_id) = matched {
        info!("[{}] matches manga {} by title", scraped.url, manga_id);
    }
    Ok(matched)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(manga_id: i32, titles: &[&str], chapters: i64) -> Candidate {
        Candidate {
            manga_id,
            titles: titles.iter().map(|title| title.to_string()).collect(),
            chapters,
        }
    }

    fn titles(titles: &[&str]) -> Vec<String> {
        titles.iter().map(|title| title.to_string()).collect()
    }

    #[test]
    fn normalizes_titles() {
        assert_eq!(normalize_title("  Solo   Leveling! "), "solo leveling");
        assert_eq!(
            normalize_title("The Beginning After The End"),
            "beginning after the end"
        );
        assert_eq!(
            normalize_title("Omniscient Reader's Viewpoint"),
            "omniscient reader s viewpoint"
        );
        assert_eq!(normalize_title("The"), "the");
        assert_eq!(normalize_title("나 혼자만 레벨업"), "나 혼자만 레벨업");
        assert_eq!(normalize_title("---"), "");
    }

    #[test]
    fn matches_on_any_title() {
        let candidates = [
            candidate(1, &["Solo Leveling", "Na Honjaman Level Up"], 200),
            candidate(2, &["Omniscient Reader"], 100),
        ];
        assert_eq!(pick_match(&titles(&["solo leveling"]), 180, &candidates), Some(1));
        assert_eq!(
            pick_match(&titles(&["Something", "Na Honjaman Level-Up"]), 180, &candidates),
            Some(1)
        );
    }

    #[test]
    fn keeps_sequels_apart() {
        let candidates = [candidate(1, &["Solo Leveling"], 200)];
        assert_eq!(pick_match(&titles(&["Solo Leveling: Ragnarok"]), 30, &candidates), None);
        assert_eq!(pick_match(&titles(&["Solo Leveling 2"]), 30, &candidates), None);
    }

    #[test]
    fn rejects_ambiguous_matches() {
        let candidates = [candidate(1, &["Return"], 50), candidate(2, &["Return"], 50)];
        assert_eq!(pick_match(&titles(&["Return"]), 50, &candidates), None);
    }

    #[test]
    fn uses_the_largest_source_of_a_manga() {
        let candidates = [
            candidate(1, &["Tower of God"], 10),
            candidate(1, &["Tower of God"], 600),
        ];
        assert_eq!(pick_match(&titles(&["Tower of God"]), 550, &candidates), Some(1));
    }

    #[test]
    fn rejects_far_apart_chapter_counts() {
        let candidates = [candidate(1, &["Return"], 300)];
        assert_eq!(pick_match(&titles(&["Return"]), 40, &candidates), None);
        assert_eq!(pick_match(&titles(&["Return"]), 5, &candidates), Some(1));
        assert_eq!(pick_match(&titles(&["Return"]), 90, &candidates), Some(1));
    }
}
