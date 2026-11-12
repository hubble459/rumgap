//! Everything behind the statistics page: one call that sums up a user's library, their reading
//! activity (from `chapter_offset`, one row per chapter they opened) and how they compare with the
//! people they follow.
//!
//! `chapter_offset` only exists for chapters opened in the reader, while `reading.progress` also
//! counts chapters marked read, so totals (time, pages, distance) are extrapolated from
//! `progress` with the per-chapter rates measured from the offsets and the stored page images.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Timelike, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, QueryResult, Statement};
use tonic::Status;

use crate::data::manga::source_cover_url;
use crate::proto::{DetailedStatsReply, StatsCount, StatsDay, StatsFriend, StatsSeries, StatsYear};

/// A series counts as dropped once it's unfinished and untouched for this long.
const DROPPED_AFTER_DAYS: i64 = 60;
/// Opens further apart than this start a new reading session.
const SESSION_GAP_MINUTES: i64 = 20;
/// Bounds on how long one chapter is assumed to take, so a chapter left open overnight (or
/// reopened months later, which bumps `updated_at`) doesn't count as hours of reading.
const MIN_CHAPTER_MINUTES: i64 = 2;
const MAX_CHAPTER_MINUTES: i64 = 30;
/// Used until there are enough timed chapters to measure the user's own pace.
const DEFAULT_MINUTES_PER_CHAPTER: f64 = 4.0;
const MIN_TIMED_CHAPTERS: usize = 5;
/// Used for sources without any stored page images.
const DEFAULT_PAGES_PER_CHAPTER: f64 = 20.0;
const DEFAULT_PAGE_RATIO: f64 = 1.4;
const MAX_FRIENDS: i64 = 20;

fn internal<E: ToString>(e: E) -> Status {
    Status::internal(e.to_string())
}

async fn query(db: &DatabaseConnection, sql: &str, values: Vec<sea_orm::Value>) -> Result<Vec<QueryResult>, Status> {
    db.query_all_raw(Statement::from_sql_and_values(db.get_database_backend(), sql, values))
        .await
        .map_err(internal)
}

/// One series in the user's library, seen through the source they read it on.
struct Series {
    manga_id: i32,
    progress: i64,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
    title: String,
    genres: Vec<String>,
    authors: Vec<String>,
    hostname: String,
    language: String,
    is_ongoing: bool,
    cover_source_id: Option<i32>,
    total: i64,
    pages_per_chapter: Option<f64>,
    screens_per_chapter: Option<f64>,
    first_opened_at: Option<NaiveDateTime>,
    last_opened_at: Option<NaiveDateTime>,
}

impl Series {
    fn is_completed(&self) -> bool {
        self.total > 0 && self.progress >= self.total
    }

    /// `reading.updated_at` alone isn't trustworthy - migrations have bulk-touched it - so the
    /// last chapter scrolled wins when there is one.
    fn last_activity(&self) -> NaiveDateTime {
        self.last_opened_at.unwrap_or(self.updated_at)
    }

    fn is_dropped(&self, now: NaiveDateTime) -> bool {
        !self.is_completed() && self.progress > 0 && now - self.last_activity() > Duration::days(DROPPED_AFTER_DAYS)
    }

    fn reply(&self, amount: i64, at: NaiveDateTime) -> StatsSeries {
        StatsSeries {
            manga_id: self.manga_id,
            title: self.title.clone(),
            cover: self.cover_source_id.map(source_cover_url),
            progress: self.progress,
            total: self.total,
            amount,
            at: millis(at),
        }
    }
}

/// One opened chapter.
struct Open {
    manga_id: i32,
    title: String,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
}

impl Open {
    /// How long this chapter is assumed to have taken.
    fn minutes(&self) -> i64 {
        (self.updated_at - self.created_at)
            .num_minutes()
            .clamp(MIN_CHAPTER_MINUTES, MAX_CHAPTER_MINUTES)
    }
}

fn millis(at: NaiveDateTime) -> i64 {
    at.and_utc().timestamp_millis()
}

fn epoch() -> NaiveDate {
    NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()
}

fn epoch_day(date: NaiveDate) -> i32 {
    (date - epoch()).num_days() as i32
}

async fn library(db: &DatabaseConnection, user: &entity::user::Model) -> Result<Vec<Series>, Status> {
    // The source picked here matches `DISPLAY_SOURCE_QUERY` in `service::v1::manga`, so titles
    // and chapter counts are the ones the user sees in their list.
    let rows = query(
        db,
        r#"
        WITH lib AS (
            SELECT r.manga_id, r.progress, r.created_at, r.updated_at,
                   src.id AS source_id, src.title, src.genres, src.authors, src.hostname, src.language, src.is_ongoing,
                   CASE WHEN src.cover_source_url IS NOT NULL THEN src.id ELSE (
                       SELECT p.id FROM manga_source p
                       WHERE p.manga_id = r.manga_id AND p.is_primary AND p.cover_source_url IS NOT NULL
                       LIMIT 1
                   ) END AS cover_source_id,
                   (SELECT COUNT(*) FROM chapter c WHERE c.manga_source_id = src.id) AS total
            FROM reading r
            CROSS JOIN LATERAL (
                SELECT * FROM manga_source s
                WHERE s.manga_id = r.manga_id
                ORDER BY (s.id = r.preferred_source_id) DESC NULLS LAST,
                         array_position($2::text[], s.hostname::text) NULLS LAST,
                         s.is_primary DESC,
                         s.id
                LIMIT 1
            ) src
            WHERE r.user_id = $1
        )
        SELECT lib.manga_id, lib.progress, lib.created_at, lib.updated_at, lib.title, lib.genres, lib.authors,
               lib.hostname, lib.language, lib.is_ongoing, lib.cover_source_id, lib.total,
               pages.pages_per_chapter, pages.screens_per_chapter, opened.first_at, opened.last_at
        FROM lib
        LEFT JOIN LATERAL (
            SELECT COUNT(*)::float8 / NULLIF(COUNT(DISTINCT ci.chapter_id), 0) AS pages_per_chapter,
                   SUM(COALESCE(ci.height::float8 / NULLIF(ci.width, 0), $3))
                       / NULLIF(COUNT(DISTINCT ci.chapter_id), 0) AS screens_per_chapter
            FROM chapter c
            JOIN chapter_image ci ON ci.chapter_id = c.id
            WHERE c.manga_source_id = lib.source_id
        ) pages ON true
        LEFT JOIN LATERAL (
            SELECT MIN(co.created_at) AS first_at, MAX(co.updated_at) AS last_at
            FROM chapter_offset co
            JOIN chapter c ON c.id = co.chapter_id
            JOIN manga_source s ON s.id = c.manga_source_id
            WHERE co.user_id = $1 AND s.manga_id = lib.manga_id
        ) opened ON true
        "#,
        vec![
            user.id.into(),
            user.preferred_hostnames.clone().into(),
            DEFAULT_PAGE_RATIO.into(),
        ],
    )
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(Series {
                manga_id: row.try_get("", "manga_id")?,
                progress: row.try_get::<i32>("", "progress")? as i64,
                created_at: row.try_get("", "created_at")?,
                updated_at: row.try_get("", "updated_at")?,
                title: row.try_get("", "title")?,
                genres: row.try_get("", "genres")?,
                authors: row.try_get("", "authors")?,
                hostname: row.try_get("", "hostname")?,
                language: row.try_get("", "language")?,
                is_ongoing: row.try_get("", "is_ongoing")?,
                cover_source_id: row.try_get("", "cover_source_id")?,
                total: row.try_get("", "total")?,
                pages_per_chapter: row.try_get("", "pages_per_chapter")?,
                screens_per_chapter: row.try_get("", "screens_per_chapter")?,
                first_opened_at: row.try_get("", "first_at")?,
                last_opened_at: row.try_get("", "last_at")?,
            })
        })
        .collect::<Result<_, sea_orm::DbErr>>()
        .map_err(internal)
}

async fn opens(db: &DatabaseConnection, user_id: i32) -> Result<Vec<Open>, Status> {
    let rows = query(
        db,
        r#"
        SELECT s.manga_id, s.title, co.created_at, co.updated_at
        FROM chapter_offset co
        JOIN chapter c ON c.id = co.chapter_id
        JOIN manga_source s ON s.id = c.manga_source_id
        WHERE co.user_id = $1
        ORDER BY co.created_at
        "#,
        vec![user_id.into()],
    )
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(Open {
                manga_id: row.try_get("", "manga_id")?,
                title: row.try_get("", "title")?,
                created_at: row.try_get("", "created_at")?,
                updated_at: row.try_get("", "updated_at")?,
            })
        })
        .collect::<Result<_, sea_orm::DbErr>>()
        .map_err(internal)
}

/// Current and longest run of consecutive days in `days`, the current one counting from today
/// or, when nothing was read yet today, from yesterday.
pub fn streaks(days: &HashSet<i32>, today: i32) -> (i32, i32) {
    let mut sorted: Vec<i32> = days.iter().copied().collect();
    sorted.sort_unstable();

    let mut longest = 0;
    let mut run = 0;
    let mut previous = None;
    for day in sorted {
        run = if previous == Some(day - 1) { run + 1 } else { 1 };
        longest = longest.max(run);
        previous = Some(day);
    }

    let mut current = 0;
    let mut day = if days.contains(&today) { today } else { today - 1 };
    while days.contains(&day) {
        current += 1;
        day -= 1;
    }

    (current, longest)
}

/// Splits opens (sorted by `created_at`) into sessions: (start, end, chapters).
pub fn sessions(opens: &[(NaiveDateTime, i64)]) -> Vec<(NaiveDateTime, NaiveDateTime, i64)> {
    let mut sessions: Vec<(NaiveDateTime, NaiveDateTime, i64)> = Vec::new();
    for &(start, minutes) in opens {
        let end = start + Duration::minutes(minutes);
        match sessions.last_mut() {
            Some(session) if start <= session.1 + Duration::minutes(SESSION_GAP_MINUTES) => {
                session.1 = session.1.max(end);
                session.2 += 1;
            }
            _ => sessions.push((start, end, 1)),
        }
    }
    sessions
}

/// The user's typical minutes per chapter: the median of the chapters with a believable
/// duration, or a default until there are enough of them.
pub fn minutes_per_chapter(durations: &[i64]) -> f64 {
    let mut timed: Vec<i64> = durations
        .iter()
        .copied()
        .filter(|&minutes| (1..=MAX_CHAPTER_MINUTES * 2).contains(&minutes))
        .collect();
    if timed.len() < MIN_TIMED_CHAPTERS {
        return DEFAULT_MINUTES_PER_CHAPTER;
    }
    timed.sort_unstable();
    let middle = timed.len() / 2;
    let median = if timed.len() % 2 == 0 {
        (timed[middle - 1] + timed[middle]) as f64 / 2.0
    } else {
        timed[middle] as f64
    };
    median.clamp(1.0, 15.0)
}

/// Tallies of a name across series, case-insensitively, keeping the first spelling seen.
#[derive(Default)]
struct Tally(HashMap<String, StatsCount>);

impl Tally {
    fn add(&mut self, name: &str, chapters: i64) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let entry = self.0.entry(name.to_lowercase()).or_insert_with(|| StatsCount {
            name: name.to_string(),
            ..Default::default()
        });
        entry.series += 1;
        entry.chapters += chapters;
    }

    fn top(self, limit: usize) -> Vec<StatsCount> {
        let mut counts: Vec<StatsCount> = self.0.into_values().collect();
        counts.sort_by(|a, b| {
            (b.chapters, b.series)
                .cmp(&(a.chapters, a.series))
                .then_with(|| a.name.cmp(&b.name))
        });
        counts.truncate(limit);
        counts
    }
}

pub async fn detailed(
    db: &DatabaseConnection,
    user: &entity::user::Model,
    utc_offset_minutes: i32,
) -> Result<DetailedStatsReply, Status> {
    let offset = Duration::minutes(utc_offset_minutes.clamp(-14 * 60, 14 * 60) as i64);
    let now = Utc::now().naive_utc();
    let local = |at: NaiveDateTime| at + offset;
    let today = local(now).date();

    let library = library(db, user).await?;
    let opens = opens(db, user.id).await?;
    let by_manga: HashMap<i32, &Series> = library.iter().map(|series| (series.manga_id, series)).collect();

    // Library totals
    let measured: Vec<&Series> = library.iter().filter(|s| s.pages_per_chapter.is_some()).collect();
    let measured_chapters = measured.len() as f64;
    let average_pages = if measured.is_empty() {
        DEFAULT_PAGES_PER_CHAPTER
    } else {
        measured.iter().filter_map(|s| s.pages_per_chapter).sum::<f64>() / measured_chapters
    };
    let average_screens = if measured.is_empty() {
        DEFAULT_PAGES_PER_CHAPTER * DEFAULT_PAGE_RATIO
    } else {
        measured.iter().filter_map(|s| s.screens_per_chapter).sum::<f64>() / measured_chapters
    };

    let chapters_read: i64 = library.iter().map(|s| s.progress).sum();
    let pages_read = library
        .iter()
        .map(|s| s.progress as f64 * s.pages_per_chapter.unwrap_or(average_pages))
        .sum::<f64>();
    let screens_scrolled = library
        .iter()
        .map(|s| s.progress as f64 * s.screens_per_chapter.unwrap_or(average_screens))
        .sum::<f64>();

    let durations: Vec<i64> = opens.iter().map(|open| (open.updated_at - open.created_at).num_minutes()).collect();
    let minutes_per_chapter = minutes_per_chapter(&durations);

    // Activity
    let mut per_day: HashMap<i32, HashMap<i32, i64>> = HashMap::new();
    let mut hours = vec![0i64; 24];
    let mut weekdays = vec![0i64; 7];
    for open in &opens {
        let at = local(open.created_at);
        *per_day
            .entry(epoch_day(at.date()))
            .or_default()
            .entry(open.manga_id)
            .or_default() += 1;
        hours[at.hour() as usize] += 1;
        weekdays[at.weekday().num_days_from_monday() as usize] += 1;
    }

    let mut days: Vec<StatsDay> = per_day
        .iter()
        .map(|(&day, series)| StatsDay {
            day,
            chapters: series.values().sum(),
        })
        .collect();
    days.sort_by_key(|day| day.day);

    let active_days: HashSet<i32> = per_day.keys().copied().collect();
    let (current_streak, longest_streak) = streaks(&active_days, epoch_day(today));

    let timed: Vec<(NaiveDateTime, i64)> = opens.iter().map(|open| (open.created_at, open.minutes())).collect();
    let all_sessions = sessions(&timed);
    let longest_session_minutes = all_sessions
        .iter()
        .map(|(start, end, _)| (*end - *start).num_minutes())
        .max()
        .unwrap_or_default();

    let title_of = |manga_id: i32| {
        opens
            .iter()
            .find(|open| open.manga_id == manga_id)
            .map(|open| open.title.clone())
            .unwrap_or_default()
    };
    let series_reply = |manga_id: i32, amount: i64, at: NaiveDateTime| match by_manga.get(&manga_id) {
        Some(series) => series.reply(amount, at),
        None => StatsSeries {
            manga_id,
            title: title_of(manga_id),
            amount,
            at: millis(at),
            ..Default::default()
        },
    };
    // The day itself, as a UTC midnight, so the client shows the date the user read on.
    let day_at = |day: i32| (epoch() + Duration::days(day as i64)).and_hms_opt(0, 0, 0).unwrap();

    let biggest_binge = per_day
        .iter()
        .max_by_key(|(&day, series)| (series.values().sum::<i64>(), day))
        .and_then(|(&day, series)| {
            let (&manga_id, _) = series.iter().max_by_key(|(&id, &count)| (count, -id))?;
            Some(series_reply(manga_id, series.values().sum(), day_at(day)))
        });

    let first_read_at = opens
        .iter()
        .map(|open| open.created_at)
        .chain(library.iter().map(|s| s.created_at))
        .min()
        .map(millis);
    let chapters_30d = opens
        .iter()
        .filter(|open| now - open.created_at <= Duration::days(30))
        .count() as i64;

    // Taste
    let mut genres = Tally::default();
    let mut authors = Tally::default();
    let mut hosts = Tally::default();
    let mut languages = Tally::default();
    for series in &library {
        let mut seen = HashSet::new();
        for genre in &series.genres {
            if seen.insert(genre.trim().to_lowercase()) {
                genres.add(genre, series.progress);
            }
        }
        for author in &series.authors {
            authors.add(author, series.progress);
        }
        hosts.add(&series.hostname, series.progress);
        languages.add(&series.language, series.progress);
    }

    // Library highlights
    let mut most_read: Vec<&Series> = library.iter().filter(|s| s.progress > 0).collect();
    most_read.sort_by_key(|s| (-s.progress, s.manga_id));

    let mut finished: Vec<&Series> = library.iter().filter(|s| s.is_completed()).collect();
    finished.sort_by_key(|s| std::cmp::Reverse(s.last_activity()));

    let mut abandoned: Vec<&Series> = library.iter().filter(|s| s.is_dropped(now)).collect();
    abandoned.sort_by_key(|s| s.last_activity());

    let fastest_finish = finished
        .iter()
        .filter(|s| s.total >= 10)
        .filter_map(|s| {
            let start = s.first_opened_at.unwrap_or(s.created_at).min(s.created_at);
            let took = s.last_activity() - start;
            (took > Duration::zero()).then_some((took, *s))
        })
        .min_by_key(|(took, s)| (*took, s.manga_id))
        .map(|(took, s)| s.reply((took.num_hours() + 23) / 24, s.last_activity()));

    let longest_relationship = library
        .iter()
        .filter(|s| s.progress > 0)
        .min_by_key(|s| (s.created_at, s.manga_id))
        .map(|s| s.reply((now - s.created_at).num_days(), s.created_at));

    let year = year_stats(&library, &opens, &by_manga, local, today.year(), minutes_per_chapter, &series_reply);
    let friends = friends(db, user, &library, local, epoch_day(today), now).await?;

    Ok(DetailedStatsReply {
        chapters_read,
        chapters_opened: opens.len() as i64,
        pages_read: pages_read.round() as i64,
        screens_scrolled,
        minutes_read: (chapters_read as f64 * minutes_per_chapter).round() as i64,
        minutes_per_chapter,
        series_total: library.len() as i64,
        series_completed: library.iter().filter(|s| s.is_completed()).count() as i64,
        series_in_progress: library
            .iter()
            .filter(|s| s.progress > 0 && !s.is_completed() && !s.is_dropped(now))
            .count() as i64,
        series_not_started: library.iter().filter(|s| s.progress == 0).count() as i64,
        series_dropped: library.iter().filter(|s| s.is_dropped(now)).count() as i64,
        series_ongoing: library.iter().filter(|s| s.is_ongoing).count() as i64,
        backlog_chapters: library.iter().map(|s| (s.total - s.progress).max(0)).sum(),
        library_chapters: library.iter().map(|s| s.total).sum(),
        days,
        hours,
        weekdays,
        current_streak,
        longest_streak,
        sessions: all_sessions.len() as i64,
        longest_session_minutes,
        biggest_binge,
        first_read_at,
        member_since: millis(user.created_at),
        chapters_30d,
        genres: genres.top(15),
        authors: authors.top(10),
        hosts: hosts.top(10),
        languages: languages.top(10),
        most_read: most_read.iter().take(5).map(|s| s.reply(s.progress, s.last_activity())).collect(),
        recently_finished: finished.iter().take(5).map(|s| s.reply(s.total, s.last_activity())).collect(),
        abandoned: abandoned
            .iter()
            .take(3)
            .map(|s| s.reply((now - s.last_activity()).num_days(), s.last_activity()))
            .collect(),
        fastest_finish,
        longest_relationship,
        friends,
        year: Some(year),
    })
}

fn year_stats(
    library: &[Series],
    opens: &[Open],
    by_manga: &HashMap<i32, &Series>,
    local: impl Fn(NaiveDateTime) -> NaiveDateTime,
    year: i32,
    minutes_per_chapter: f64,
    series_reply: &impl Fn(i32, i64, NaiveDateTime) -> StatsSeries,
) -> StatsYear {
    let this_year: Vec<&Open> = opens.iter().filter(|open| local(open.created_at).year() == year).collect();

    let mut months = vec![0i64; 12];
    let mut per_series: HashMap<i32, i64> = HashMap::new();
    let mut genres = Tally::default();
    let mut days = HashSet::new();
    for open in &this_year {
        let at = local(open.created_at);
        months[at.month0() as usize] += 1;
        *per_series.entry(open.manga_id).or_default() += 1;
        days.insert(at.date());
        if let Some(series) = by_manga.get(&open.manga_id) {
            for genre in &series.genres {
                genres.add(genre, 1);
            }
        }
    }

    let top_series = per_series
        .iter()
        .max_by_key(|(&id, &count)| (count, -id))
        .map(|(&id, &count)| {
            let last = this_year
                .iter()
                .filter(|open| open.manga_id == id)
                .map(|open| open.updated_at)
                .max()
                .unwrap_or_default();
            series_reply(id, count, last)
        });
    let (top_month, top_month_chapters) = months
        .iter()
        .enumerate()
        .filter(|(_, &count)| count > 0)
        .max_by_key(|(month, &count)| (count, -(*month as i64)))
        .map_or((0, 0), |(month, &count)| (month as i32 + 1, count));

    StatsYear {
        year,
        chapters: this_year.len() as i64,
        minutes: (this_year.len() as f64 * minutes_per_chapter).round() as i64,
        new_series: library.iter().filter(|s| local(s.created_at).year() == year).count() as i64,
        finished_series: library
            .iter()
            .filter(|s| s.is_completed() && local(s.last_activity()).year() == year)
            .count() as i64,
        active_days: days.len() as i64,
        top_series,
        top_genre: genres.top(1).into_iter().next().map(|genre| genre.name),
        top_month,
        top_month_chapters,
        months,
    }
}

/// The user and everyone they follow, most chapters this month first.
async fn friends(
    db: &DatabaseConnection,
    user: &entity::user::Model,
    library: &[Series],
    local: impl Fn(NaiveDateTime) -> NaiveDateTime,
    today: i32,
    now: NaiveDateTime,
) -> Result<Vec<StatsFriend>, Status> {
    let followed = query(
        db,
        r#"
        SELECT u.id, u.username
        FROM friend f
        JOIN "user" u ON u.id = f.friend_id
        WHERE f.user_id = $1 AND u.id <> $1
        ORDER BY u.username
        LIMIT $2
        "#,
        vec![user.id.into(), MAX_FRIENDS.into()],
    )
    .await?;
    if followed.is_empty() {
        return Ok(Vec::new());
    }

    let mut people: Vec<(i32, String)> = vec![(user.id, user.username.clone())];
    for row in followed {
        people.push((
            row.try_get("", "id").map_err(internal)?,
            row.try_get("", "username").map_err(internal)?,
        ));
    }
    let ids: Vec<i32> = people.iter().map(|(id, _)| *id).collect();

    let mut days: HashMap<i32, HashSet<i32>> = HashMap::new();
    let mut recent: HashMap<i32, i64> = HashMap::new();
    for row in query(
        db,
        "SELECT user_id, created_at FROM chapter_offset WHERE user_id = ANY($1::int4[])",
        vec![ids.clone().into()],
    )
    .await?
    {
        let user_id: i32 = row.try_get("", "user_id").map_err(internal)?;
        let created_at: NaiveDateTime = row.try_get("", "created_at").map_err(internal)?;
        days.entry(user_id).or_default().insert(epoch_day(local(created_at).date()));
        if now - created_at <= Duration::days(30) {
            *recent.entry(user_id).or_default() += 1;
        }
    }

    let mut totals: HashMap<i32, i64> = HashMap::new();
    for row in query(
        db,
        "SELECT user_id, SUM(progress)::int8 AS total FROM reading WHERE user_id = ANY($1::int4[]) GROUP BY user_id",
        vec![ids.clone().into()],
    )
    .await?
    {
        totals.insert(
            row.try_get("", "user_id").map_err(internal)?,
            row.try_get("", "total").map_err(internal)?,
        );
    }

    let mine: HashMap<i32, &Series> = library.iter().map(|s| (s.manga_id, s)).collect();
    let mut shared: HashMap<i32, Vec<&Series>> = HashMap::new();
    for row in query(
        db,
        "SELECT user_id, manga_id FROM reading WHERE user_id = ANY($1::int4[]) AND user_id <> $2",
        vec![ids.clone().into(), user.id.into()],
    )
    .await?
    {
        let manga_id: i32 = row.try_get("", "manga_id").map_err(internal)?;
        if let Some(series) = mine.get(&manga_id) {
            shared
                .entry(row.try_get("", "user_id").map_err(internal)?)
                .or_default()
                .push(series);
        }
    }

    let mut friends: Vec<StatsFriend> = people
        .into_iter()
        .map(|(id, username)| {
            let (current_streak, longest_streak) = streaks(days.get(&id).unwrap_or(&HashSet::new()), today);
            let mut together = shared.remove(&id).unwrap_or_default();
            together.sort_by_key(|s| (-s.progress, s.manga_id));
            StatsFriend {
                user_id: id,
                username,
                is_me: id == user.id,
                chapters_30d: recent.get(&id).copied().unwrap_or_default(),
                chapters_total: totals.get(&id).copied().unwrap_or_default(),
                current_streak,
                longest_streak,
                shared_count: together.len() as i64,
                shared_titles: together.iter().take(3).map(|s| s.title.clone()).collect(),
            }
        })
        .collect();
    friends.sort_by_key(|f| (-f.chapters_30d, -f.chapters_total, f.user_id));
    Ok(friends)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes: i64) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap() + Duration::minutes(minutes)
    }

    #[test]
    fn streaks_count_from_yesterday_when_today_is_empty() {
        let days: HashSet<i32> = [1, 2, 3, 7, 8, 9, 10].into();
        assert_eq!(streaks(&days, 11), (4, 4));
        assert_eq!(streaks(&days, 10), (4, 4));
        assert_eq!(streaks(&days, 12), (0, 4));
        assert_eq!(streaks(&HashSet::new(), 5), (0, 0));
    }

    #[test]
    fn sessions_split_on_long_gaps() {
        let opens = [(at(0), 5), (at(6), 5), (at(30), 2), (at(200), 10)];
        let sessions = sessions(&opens);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0], (at(0), at(32), 3));
        assert_eq!(sessions[1], (at(200), at(210), 1));
    }

    #[test]
    fn minutes_per_chapter_uses_the_median_of_believable_durations() {
        assert_eq!(minutes_per_chapter(&[3, 4]), DEFAULT_MINUTES_PER_CHAPTER);
        assert_eq!(minutes_per_chapter(&[0, 3, 4, 5, 6, 7, 50_000]), 5.0);
        assert_eq!(minutes_per_chapter(&[2, 2, 3, 3, 4, 4]), 3.0);
    }

    #[test]
    fn tally_merges_case_and_ranks_by_chapters() {
        let mut tally = Tally::default();
        tally.add("Action", 10);
        tally.add(" action ", 5);
        tally.add("Romance", 20);
        tally.add("", 100);
        let top = tally.top(5);
        assert_eq!(top.len(), 2);
        assert_eq!((top[0].name.as_str(), top[0].series, top[0].chapters), ("Romance", 1, 20));
        assert_eq!((top[1].name.as_str(), top[1].series, top[1].chapters), ("Action", 2, 15));
    }
}
