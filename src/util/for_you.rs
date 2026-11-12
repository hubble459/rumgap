//! Ranks the manga a user isn't reading by how well they fit what the user reads, for the
//! "for you" swipe deck. Content-based only: every manga becomes a TF-IDF vector of its
//! genres and description/title keywords (from all of its sources), the user's reading list
//! becomes a weighted "liked" profile and their recent dismissals a "disliked" one.
//! With ~4 active readers, collaborative filtering would be noise.

use std::collections::{HashMap, HashSet};

use chrono::{NaiveDateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use tonic::Status;

/// Genres say more about a manga than any one keyword does.
const GENRE_WEIGHT: f64 = 2.0;
/// How much a dismissed manga's profile pulls scores down, against the liked profile's 1.0.
const DISLIKE_WEIGHT: f64 = 0.5;
/// A bit of popularity (number of readers) so a cold profile still has an order.
const POPULARITY_WEIGHT: f64 = 0.05;
/// How strongly the diversity re-ranking pushes away from manga already ranked above.
const DIVERSITY_WEIGHT: f64 = 0.3;
/// Only the best this many are re-ranked for diversity; the rest keep their score order.
const DIVERSITY_POOL: usize = 200;
/// A reading's weight halves every this many days since it was last updated.
const READING_HALF_LIFE_DAYS: f64 = 180.0;
/// A reading without progress (e.g. just swiped right) still counts a little.
const READING_BASE_WEIGHT: f64 = 0.3;

type Vector = HashMap<String, f64>;

fn normalize(vector: &mut Vector) {
    let norm = vector.values().map(|value| value * value).sum::<f64>().sqrt();
    if norm > 0.0 {
        vector.values_mut().for_each(|value| *value /= norm);
    }
}

fn dot(a: &Vector, b: &Vector) -> f64 {
    let (small, large) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    small
        .iter()
        .filter_map(|(feature, value)| large.get(feature).map(|other| value * other))
        .sum()
}

/// Unit-length TF-IDF vectors (binary term frequency) per manga. Features only one manga has
/// are dropped: they can't make two manga similar.
pub fn build_vectors(features: &HashMap<i32, HashSet<String>>) -> HashMap<i32, Vector> {
    let total = features.len() as f64;
    let mut document_frequency: HashMap<&str, f64> = HashMap::new();
    for manga_features in features.values() {
        for feature in manga_features {
            *document_frequency.entry(feature).or_default() += 1.0;
        }
    }

    features
        .iter()
        .map(|(manga_id, manga_features)| {
            let mut vector: Vector = manga_features
                .iter()
                .filter_map(|feature| {
                    let df = document_frequency[feature.as_str()];
                    if df < 2.0 {
                        return None;
                    }
                    let weight = if feature.starts_with("g:") { GENRE_WEIGHT } else { 1.0 };
                    Some((feature.clone(), weight * (total / df).ln()))
                })
                .filter(|(_, value)| *value > 0.0)
                .collect();
            normalize(&mut vector);
            (*manga_id, vector)
        })
        .collect()
}

/// Unit-length weighted sum of the vectors of `weighted` manga.
fn profile(vectors: &HashMap<i32, Vector>, weighted: &[(i32, f64)]) -> Vector {
    let mut profile = Vector::new();
    for (manga_id, weight) in weighted {
        if let Some(vector) = vectors.get(manga_id) {
            for (feature, value) in vector {
                *profile.entry(feature.clone()).or_default() += weight * value;
            }
        }
    }
    normalize(&mut profile);
    profile
}

/// `candidates` best first: similarity to the liked profile, minus some similarity to the
/// disliked one, plus a little popularity; then the top is re-ranked so one kind of manga
/// doesn't fill the deck (maximal marginal relevance).
pub fn rank(
    vectors: &HashMap<i32, Vector>,
    liked: &[(i32, f64)],
    disliked: &[(i32, f64)],
    popularity: &HashMap<i32, f64>,
    candidates: &[i32],
) -> Vec<i32> {
    let liked = profile(vectors, liked);
    let disliked = profile(vectors, disliked);
    let most_popular = popularity.values().copied().fold(0.0, f64::max).max(1.0);
    let empty = Vector::new();

    let mut scored: Vec<(i32, f64)> = candidates
        .iter()
        .map(|manga_id| {
            let vector = vectors.get(manga_id).unwrap_or(&empty);
            let score = dot(vector, &liked) - DISLIKE_WEIGHT * dot(vector, &disliked)
                + POPULARITY_WEIGHT * popularity.get(manga_id).copied().unwrap_or(0.0) / most_popular;
            (*manga_id, score)
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));

    let rest = scored.split_off(scored.len().min(DIVERSITY_POOL));
    let mut pool = scored;
    let mut ranked: Vec<i32> = Vec::with_capacity(candidates.len());
    let mut closest: Vec<f64> = vec![0.0; pool.len()];
    while !pool.is_empty() {
        let (best, _) = pool
            .iter()
            .zip(&closest)
            .enumerate()
            .map(|(index, ((_, score), close))| (index, score - DIVERSITY_WEIGHT * close))
            .fold(
                (0, f64::NEG_INFINITY),
                |best, next| if next.1 > best.1 { next } else { best },
            );
        let (picked, _) = pool.remove(best);
        closest.remove(best);
        let picked_vector = vectors.get(&picked).unwrap_or(&empty);
        for ((manga_id, _), close) in pool.iter().zip(closest.iter_mut()) {
            *close = close.max(dot(vectors.get(manga_id).unwrap_or(&empty), picked_vector));
        }
        ranked.push(picked);
    }
    ranked.extend(rest.into_iter().map(|(manga_id, _)| manga_id));
    ranked
}

pub fn dismiss_days() -> i64 {
    std::env::var("FOR_YOU_DISMISS_DAYS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(90)
}

fn reading_weight(progress: i32, updated_at: NaiveDateTime, now: NaiveDateTime) -> f64 {
    let age_days = (now - updated_at).num_seconds().max(0) as f64 / 86_400.0;
    (READING_BASE_WEIGHT + (1.0 + progress.max(0) as f64).ln()) * 0.5f64.powf(age_days / READING_HALF_LIFE_DAYS)
}

/// Fades from 1 right after the swipe to 0 when the cooldown ends.
fn dismissal_weight(created_at: NaiveDateTime, now: NaiveDateTime, cooldown_days: i64) -> f64 {
    let age_days = (now - created_at).num_seconds().max(0) as f64 / 86_400.0;
    (1.0 - age_days / cooldown_days.max(1) as f64).max(0.0)
}

fn internal<E: ToString>(e: E) -> Status {
    Status::internal(e.to_string())
}

async fn query(
    db: &DatabaseConnection,
    sql: &str,
    values: Vec<sea_orm::Value>,
) -> Result<Vec<sea_orm::QueryResult>, Status> {
    db.query_all_raw(Statement::from_sql_and_values(db.get_database_backend(), sql, values))
        .await
        .map_err(internal)
}

/// Every manga `user_id` isn't reading and hasn't dismissed within the cooldown, best match
/// first.
pub async fn ranked_for_user(db: &DatabaseConnection, user_id: i32) -> Result<Vec<i32>, Status> {
    let now = Utc::now().naive_utc();
    let cooldown_days = dismiss_days();

    let mut features: HashMap<i32, HashSet<String>> = HashMap::new();
    for row in query(db, "SELECT id FROM manga", vec![]).await? {
        features.entry(row.try_get_by_index(0).map_err(internal)?).or_default();
    }
    let feature_rows = query(
        db,
        r#"
        SELECT manga_id, 'g:' || lower(trim(genre))
        FROM manga_source, unnest(genres) AS genre
        WHERE trim(genre) <> ''
        UNION
        SELECT manga_id, 'w:' || word
        FROM (
            SELECT manga_id, unnest(tsvector_to_array(to_tsvector('english',
                coalesce(title, '') || ' ' || array_to_string(alt_titles, ' ') || ' ' || coalesce(description, '')
            ))) AS word
            FROM manga_source
        ) words
        WHERE length(word) > 2 AND word !~ '^[0-9]+$'
        "#,
        vec![],
    )
    .await?;
    for row in feature_rows {
        let manga_id: i32 = row.try_get_by_index(0).map_err(internal)?;
        let feature: String = row.try_get_by_index(1).map_err(internal)?;
        features.entry(manga_id).or_default().insert(feature);
    }

    let mut liked = vec![];
    for row in query(
        db,
        "SELECT manga_id, progress, updated_at FROM reading WHERE user_id = $1",
        vec![user_id.into()],
    )
    .await?
    {
        let manga_id: i32 = row.try_get_by_index(0).map_err(internal)?;
        let progress: i32 = row.try_get_by_index(1).map_err(internal)?;
        let updated_at: NaiveDateTime = row.try_get_by_index(2).map_err(internal)?;
        liked.push((manga_id, reading_weight(progress, updated_at, now)));
    }

    let mut disliked = vec![];
    for row in query(
        db,
        "SELECT manga_id, created_at FROM manga_dismissal WHERE user_id = $1 AND created_at > $2",
        vec![user_id.into(), (now - chrono::Duration::days(cooldown_days)).into()],
    )
    .await?
    {
        let manga_id: i32 = row.try_get_by_index(0).map_err(internal)?;
        let created_at: NaiveDateTime = row.try_get_by_index(1).map_err(internal)?;
        disliked.push((manga_id, dismissal_weight(created_at, now, cooldown_days)));
    }

    let mut popularity = HashMap::new();
    for row in query(db, "SELECT manga_id, count(*) FROM reading GROUP BY manga_id", vec![]).await? {
        let manga_id: i32 = row.try_get_by_index(0).map_err(internal)?;
        let readers: i64 = row.try_get_by_index(1).map_err(internal)?;
        popularity.insert(manga_id, readers as f64);
    }

    let excluded: HashSet<i32> = liked.iter().chain(&disliked).map(|(manga_id, _)| *manga_id).collect();
    let mut candidates: Vec<i32> = features
        .keys()
        .copied()
        .filter(|manga_id| !excluded.contains(manga_id))
        .collect();
    candidates.sort();

    let vectors = build_vectors(&features);
    Ok(rank(&vectors, &liked, &disliked, &popularity, &candidates))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn features(entries: &[(i32, &[&str])]) -> HashMap<i32, HashSet<String>> {
        entries
            .iter()
            .map(|(manga_id, features)| (*manga_id, features.iter().map(|feature| feature.to_string()).collect()))
            .collect()
    }

    fn catalog() -> HashMap<i32, Vector> {
        build_vectors(&features(&[
            (1, &["g:action", "g:murim", "w:sect", "w:regress"]),
            (2, &["g:action", "g:murim", "w:sect", "w:cultiv"]),
            (3, &["g:romance", "g:school", "w:confess"]),
            (4, &["g:romance", "g:school", "w:confess", "w:festiv"]),
            (5, &["g:action", "g:murim", "w:regress", "w:cultiv"]),
            (6, &["g:action", "w:dungeon"]),
        ]))
    }

    #[test]
    fn vectors_are_unit_length_and_skip_unique_features() {
        let vectors = catalog();
        for vector in vectors.values().filter(|vector| !vector.is_empty()) {
            assert!((dot(vector, vector) - 1.0).abs() < 1e-9);
        }
        assert!(!vectors[&4].contains_key("w:festiv"));
        assert!(!vectors[&6].contains_key("w:dungeon"));
    }

    #[test]
    fn ranks_similar_manga_first() {
        let vectors = catalog();
        let ranked = rank(&vectors, &[(1, 1.0)], &[], &HashMap::new(), &[2, 3, 4, 5, 6]);
        assert!(ranked.iter().position(|id| *id == 2) < ranked.iter().position(|id| *id == 3));
        assert!(ranked.iter().position(|id| *id == 5) < ranked.iter().position(|id| *id == 4));
        assert_eq!(ranked.len(), 5);
    }

    #[test]
    fn dismissals_push_similar_manga_down() {
        let vectors = catalog();
        let without = rank(&vectors, &[], &[], &HashMap::new(), &[2, 4]);
        let with = rank(&vectors, &[], &[(3, 1.0)], &HashMap::new(), &[2, 4]);
        assert_eq!(with, vec![2, 4]);
        assert_eq!(without.len(), 2);
    }

    #[test]
    fn cold_start_uses_popularity() {
        let vectors = catalog();
        let popularity = HashMap::from([(4, 3.0), (2, 1.0)]);
        assert_eq!(rank(&vectors, &[], &[], &popularity, &[2, 3, 4]), vec![4, 2, 3]);
    }

    #[test]
    fn diversity_breaks_up_runs_of_the_same_kind() {
        let vectors = build_vectors(&features(&[
            (1, &["g:action", "g:murim", "w:sect"]),
            (2, &["g:action", "g:murim", "w:sect"]),
            (3, &["g:action", "g:murim", "w:sect"]),
            (4, &["g:romance", "w:confess"]),
            (5, &["g:romance", "w:confess"]),
        ]));
        let liked = [(1, 1.1), (4, 1.0)];
        let ranked = rank(&vectors, &liked, &[], &HashMap::new(), &[2, 3, 5]);
        assert_eq!(ranked, vec![2, 5, 3]);
    }

    #[test]
    fn weights_fade_with_age() {
        let now = Utc::now().naive_utc();
        let fresh = reading_weight(10, now, now);
        let old = reading_weight(10, now - chrono::Duration::days(180), now);
        assert!((old - fresh / 2.0).abs() < 1e-9);
        assert!(reading_weight(0, now, now) > 0.0);

        assert_eq!(dismissal_weight(now, now, 90), 1.0);
        assert!((dismissal_weight(now - chrono::Duration::days(45), now, 90) - 0.5).abs() < 1e-9);
        assert_eq!(dismissal_weight(now - chrono::Duration::days(100), now, 90), 0.0);
    }
}
