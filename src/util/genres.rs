//! One spelling per genre, whichever site it was scraped from: "Sci Fi", "sci-fi" and a split
//! "Sci" + "fi" all become "Sci-Fi", "Vampires" becomes "Vampire", "Slice Of Life" becomes
//! "Slice of Life". Applied when a source is scraped, and to every stored source at startup
//! so changes to the rules reach existing data without a migration.

use log::{info, warn};
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

/// Genres that mean the same thing, by `key` (lowercase, letters and digits only).
const ALIASES: &[(&str, &str)] = &[
    ("sci", "Sci-Fi"),
    ("fi", "Sci-Fi"),
    ("scifi", "Sci-Fi"),
    ("sciencefiction", "Sci-Fi"),
    ("rebirth", "Reincarnation"),
    ("reborn", "Reincarnation"),
    ("reincarnated", "Reincarnation"),
    ("returner", "Reincarnation"),
    ("returned", "Reincarnation"),
    ("regression", "Reincarnation"),
    ("regressor", "Reincarnation"),
    ("anotherchance", "Reincarnation"),
    ("vampires", "Vampire"),
    ("demons", "Demon"),
    ("videogames", "Video Game"),
];

/// Tags sites list as genres that describe the format or status instead, by `key`; dropped.
const NOT_GENRES: &[&str] = &[
    "longstrip",
    "fullcolor",
    "fullcolour",
    "officialcolored",
    "fancolored",
    "webcomic",
    "webtoon",
    "webtoons",
    "completed",
    "100chapter",
];

/// Words kept lowercase inside a genre ("Slice of Life").
const SMALL_WORDS: &[&str] = &["a", "an", "and", "in", "of", "on", "the", "to"];

fn key(genre: &str) -> String {
    genre
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Capitalizes each word (keeping the rest of it as is, so "MC" stays) except small words
/// after the first.
fn title_case(genre: &str) -> String {
    genre
        .split_whitespace()
        .enumerate()
        .map(|(index, word)| {
            if index > 0 && SMALL_WORDS.contains(&word.to_lowercase().as_str()) {
                return word.to_lowercase();
            }
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Normalized, without duplicates, in their original order.
pub fn normalize(genres: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    genres
        .iter()
        .filter_map(|genre| {
            let genre = genre.trim();
            let key = key(genre);
            if key.is_empty() || NOT_GENRES.contains(&key.as_str()) {
                return None;
            }
            let normalized = match ALIASES.iter().find(|(alias, _)| *alias == key) {
                Some((_, canonical)) => canonical.to_string(),
                None => title_case(genre),
            };
            seen.insert(self::key(&normalized)).then_some(normalized)
        })
        .collect()
}

/// Normalizes the genres of every stored source whose genres aren't normalized yet.
pub async fn normalize_stored(db: &DatabaseConnection) {
    let backend = db.get_database_backend();
    let rows = match db
        .query_all_raw(Statement::from_string(backend, "SELECT id, genres FROM manga_source"))
        .await
    {
        Ok(rows) => rows,
        Err(e) => return warn!("[Genres] Couldn't load genres: {e}"),
    };

    let mut updated = 0;
    for row in rows {
        let (Ok(id), Ok(genres)) = (row.try_get_by_index::<i32>(0), row.try_get_by_index::<Vec<String>>(1)) else {
            continue;
        };
        let normalized = normalize(&genres);
        if normalized == genres {
            continue;
        }
        let update = Statement::from_sql_and_values(
            backend,
            "UPDATE manga_source SET genres = $1 WHERE id = $2",
            [normalized.into(), id.into()],
        );
        match db.execute_raw(update).await {
            Ok(_) => updated += 1,
            Err(e) => warn!("[Genres] Couldn't update source {id}: {e}"),
        }
    }
    if updated > 0 {
        info!("[Genres] Normalized the genres of {updated} source(s)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normalized(genres: &[&str]) -> Vec<String> {
        normalize(&genres.iter().map(|genre| genre.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn merges_spellings() {
        assert_eq!(normalized(&["Sci", "fi", "Action"]), ["Sci-Fi", "Action"]);
        assert_eq!(normalized(&["Sci Fi"]), ["Sci-Fi"]);
        assert_eq!(normalized(&["sci-fi"]), ["Sci-Fi"]);
        assert_eq!(normalized(&["Vampires", "Vampire"]), ["Vampire"]);
        assert_eq!(normalized(&["Video Games"]), ["Video Game"]);
        assert_eq!(normalized(&["Demons"]), ["Demon"]);
    }

    #[test]
    fn merges_reincarnation_tropes() {
        assert_eq!(
            normalized(&[
                "Rebirth",
                "Returner",
                "Returned",
                "Regression",
                "Another chance",
                "Reincarnation"
            ]),
            ["Reincarnation"]
        );
    }

    #[test]
    fn fixes_casing_but_keeps_the_rest() {
        assert_eq!(normalized(&["Slice Of Life"]), ["Slice of Life"]);
        assert_eq!(normalized(&["slice of life"]), ["Slice of Life"]);
        assert_eq!(normalized(&["martial arts"]), ["Martial Arts"]);
        assert_eq!(normalized(&["Genius MC"]), ["Genius MC"]);
        assert_eq!(normalized(&["Boys' Love", "4-Koma"]), ["Boys' Love", "4-Koma"]);
        assert_eq!(normalized(&["The Hero"]), ["The Hero"]);
    }

    #[test]
    fn drops_format_and_status_tags() {
        assert_eq!(
            normalized(&[
                "Action", "Long Strip", "Full Color", "Official Colored", "Fan Colored", "Web Comic", "Webtoon",
                "Webtoons", "Completed", "+100 Chapter", "Manhwa",
            ]),
            ["Action", "Manhwa"]
        );
    }

    #[test]
    fn drops_empty_and_duplicate_genres() {
        assert_eq!(normalized(&["", "  ", "Action", "action", " Action "]), ["Action"]);
    }
}
