//! AniList API client: OAuth (authorization-code grant, redirect to rumgap's
//! `/anilist/callback`), the GraphQL queries rumgap needs, and a shared rate limit.
//!
//! AniList tokens last a year and refresh tokens aren't supported, so an expired or
//! revoked token just disconnects the user until they connect again.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{NaiveDateTime, Utc};
use hmac::Mac;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::interceptor::auth::SECRET_KEY;

const GRAPHQL_URL: &str = "https://graphql.anilist.co";
const AUTHORIZE_URL: &str = "https://anilist.co/api/v2/oauth/authorize";
const TOKEN_URL: &str = "https://anilist.co/api/v2/oauth/token";

/// AniList allows 90 requests a minute, or 30 while degraded; its `X-RateLimit-Limit`
/// header says which. Until a response says otherwise, assume degraded.
const DEFAULT_LIMIT_PER_MINUTE: u64 = 30;
const STATE_TTL_SECS: i64 = 10 * 60;
const STATE_PREFIX: &[u8] = b"rumgap-anilist-state\n";
pub const EXPIRY_WARN_DAYS: i64 = 14;

lazy_static! {
    static ref CLIENT: reqwest::Client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("Failed to build AniList HTTP client");
    static ref NEXT_REQUEST: Mutex<Instant> = Mutex::new(Instant::now());
}
static LIMIT_PER_MINUTE: AtomicU64 = AtomicU64::new(DEFAULT_LIMIT_PER_MINUTE);

/// Spreads requests evenly over the minute, with some headroom.
fn request_gap() -> Duration {
    let limit = LIMIT_PER_MINUTE.load(Ordering::Relaxed).max(1);
    Duration::from_millis(60_000 * 11 / 10 / limit)
}

fn track_limit(headers: &reqwest::header::HeaderMap) {
    let limit = headers
        .get("x-ratelimit-limit")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if let Some(limit) = limit.filter(|l| *l > 0) {
        let previous = LIMIT_PER_MINUTE.swap(limit, Ordering::Relaxed);
        if previous != limit {
            info!("[AniList] Rate limit is now {limit} requests a minute");
        }
    }
}

#[derive(Debug)]
pub enum AnilistError {
    /// The token is expired or revoked.
    Unauthorized,
    NotConfigured,
    Other(String),
}

impl std::fmt::Display for AnilistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnilistError::Unauthorized => write!(f, "AniList token is expired or revoked"),
            AnilistError::NotConfigured => write!(f, "AniList is not configured on this server"),
            AnilistError::Other(e) => write!(f, "{e}"),
        }
    }
}

impl From<reqwest::Error> for AnilistError {
    fn from(e: reqwest::Error) -> Self {
        AnilistError::Other(e.to_string())
    }
}

impl From<AnilistError> for tonic::Status {
    fn from(e: AnilistError) -> Self {
        match e {
            AnilistError::Unauthorized => tonic::Status::unauthenticated(e.to_string()),
            AnilistError::NotConfigured => tonic::Status::failed_precondition(e.to_string()),
            AnilistError::Other(_) => tonic::Status::unavailable(format!("AniList: {e}")),
        }
    }
}

pub struct Config {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

pub fn config() -> Result<Config, AnilistError> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    Ok(Config {
        client_id: var("ANILIST_CLIENT_ID").ok_or(AnilistError::NotConfigured)?,
        client_secret: var("ANILIST_CLIENT_SECRET").ok_or(AnilistError::NotConfigured)?,
        redirect_uri: var("ANILIST_REDIRECT_URI").ok_or(AnilistError::NotConfigured)?,
    })
}

/// Waits for this request's turn under the shared rate limit.
async fn throttle() {
    let wait_until = {
        let mut next = NEXT_REQUEST.lock().await;
        let now = Instant::now();
        let at = (*next).max(now);
        *next = at + request_gap();
        at
    };
    tokio::time::sleep_until(wait_until).await;
}

/// Pushes the shared rate limit back, after AniList said to slow down.
async fn back_off(secs: u64) {
    let mut next = NEXT_REQUEST.lock().await;
    *next = (*next).max(Instant::now() + Duration::from_secs(secs));
}

async fn graphql(query: &str, variables: Value, token: Option<&str>) -> Result<Value, AnilistError> {
    for _ in 0..3 {
        throttle().await;
        let mut request = CLIENT
            .post(GRAPHQL_URL)
            .header("Accept", "application/json")
            .json(&json!({ "query": query, "variables": variables }));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await?;
        let status = response.status();
        track_limit(response.headers());

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok())
                .unwrap_or(60);
            warn!("[AniList] Rate limited, waiting {retry_after}s");
            back_off(retry_after).await;
            continue;
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(AnilistError::Unauthorized);
        }

        let body: Value = response.json().await?;
        let errors = body.get("errors").and_then(Value::as_array).cloned().unwrap_or_default();
        if errors
            .iter()
            .any(|e| e["status"].as_i64() == Some(401) || e["message"] == "Invalid token")
        {
            return Err(AnilistError::Unauthorized);
        }
        // A media that doesn't exist is a 404 error, not a failure.
        if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
            return Ok(body["data"].clone());
        }
        return Err(AnilistError::Other(format!("{status}: {}", Value::Array(errors))));
    }
    Err(AnilistError::Other("rate limited".to_string()))
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MediaTitle {
    pub romaji: Option<String>,
    pub english: Option<String>,
    pub native: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CoverImage {
    #[serde(rename = "extraLarge")]
    pub extra_large: Option<String>,
    pub large: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FuzzyDate {
    pub year: Option<i32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    pub id: i32,
    pub site_url: Option<String>,
    #[serde(default)]
    pub title: MediaTitle,
    #[serde(default)]
    pub synonyms: Vec<String>,
    pub cover_image: Option<CoverImage>,
    pub banner_image: Option<String>,
    pub average_score: Option<i32>,
    #[serde(default)]
    pub genres: Vec<String>,
    pub status: Option<String>,
    pub chapters: Option<i32>,
    pub format: Option<String>,
    pub start_date: Option<FuzzyDate>,
    pub description: Option<String>,
}

impl Media {
    pub fn url(&self) -> String {
        self.site_url
            .clone()
            .unwrap_or_else(|| format!("https://anilist.co/manga/{}", self.id))
    }

    pub fn cover(&self) -> Option<String> {
        self.cover_image
            .as_ref()
            .and_then(|c| c.extra_large.clone().or_else(|| c.large.clone()))
    }

    /// Every title AniList knows it by.
    pub fn titles(&self) -> impl Iterator<Item = &String> {
        [&self.title.romaji, &self.title.english, &self.title.native]
            .into_iter()
            .flatten()
            .chain(self.synonyms.iter())
    }

    pub fn display_title(&self) -> Option<String> {
        self.title
            .english
            .clone()
            .or_else(|| self.title.romaji.clone())
            .or_else(|| self.title.native.clone())
    }

    pub fn into_proto(self) -> crate::proto::AnilistMedia {
        crate::proto::AnilistMedia {
            id: self.id,
            url: self.url(),
            cover: self.cover(),
            title_romaji: self.title.romaji,
            title_english: self.title.english,
            title_native: self.title.native,
            synonyms: self.synonyms,
            chapters: self.chapters,
            status: self.status,
            format: self.format,
            start_year: self.start_date.and_then(|d| d.year),
        }
    }
}

const MEDIA_FIELDS: &str = "id siteUrl title { romaji english native } synonyms \
    coverImage { extraLarge large } bannerImage averageScore genres status chapters format \
    startDate { year } description(asHtml: false)";

/// Manga (not light novels) matching `query`, best match first.
pub async fn search_manga(query: &str) -> Result<Vec<Media>, AnilistError> {
    let gql = format!(
        "query ($search: String) {{ Page(perPage: 10) {{ media(search: $search, type: MANGA, \
         format_not: NOVEL, sort: SEARCH_MATCH) {{ {MEDIA_FIELDS} }} }} }}"
    );
    let data = graphql(&gql, json!({ "search": query }), None).await?;
    serde_json::from_value(data["Page"]["media"].clone()).map_err(|e| AnilistError::Other(e.to_string()))
}

pub async fn media(id: i32) -> Result<Option<Media>, AnilistError> {
    let gql = format!("query ($id: Int) {{ Media(id: $id, type: MANGA) {{ {MEDIA_FIELDS} }} }}");
    let data = graphql(&gql, json!({ "id": id }), None).await?;
    if data.is_null() || data["Media"].is_null() {
        return Ok(None);
    }
    serde_json::from_value(data["Media"].clone())
        .map(Some)
        .map_err(|e| AnilistError::Other(e.to_string()))
}

pub async fn viewer(token: &str) -> Result<(i32, String), AnilistError> {
    let data = graphql("query { Viewer { id name } }", json!({}), Some(token)).await?;
    let id = data["Viewer"]["id"]
        .as_i64()
        .ok_or_else(|| AnilistError::Other("Viewer without id".to_string()))?;
    let name = data["Viewer"]["name"].as_str().unwrap_or_default().to_string();
    Ok((id as i32, name))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    pub progress: i32,
    pub status: String,
}

/// The viewer's list entry for `media_id`, if it's on their list.
pub async fn list_entry(token: &str, media_id: i32) -> Result<Option<ListEntry>, AnilistError> {
    let data = graphql(
        "query ($id: Int) { Media(id: $id) { mediaListEntry { progress status } } }",
        json!({ "id": media_id }),
        Some(token),
    )
    .await?;
    let entry = &data["Media"]["mediaListEntry"];
    if entry.is_null() {
        return Ok(None);
    }
    Ok(Some(ListEntry {
        progress: entry["progress"].as_i64().unwrap_or(0) as i32,
        status: entry["status"].as_str().unwrap_or_default().to_string(),
    }))
}

pub async fn save_progress(token: &str, media_id: i32, entry: &ListEntry) -> Result<(), AnilistError> {
    graphql(
        "mutation ($mediaId: Int, $progress: Int, $status: MediaListStatus) { \
         SaveMediaListEntry(mediaId: $mediaId, progress: $progress, status: $status) { id } }",
        json!({ "mediaId": media_id, "progress": entry.progress, "status": entry.status }),
        Some(token),
    )
    .await?;
    Ok(())
}

/// Removes the media from the viewer's list. Nothing to do when it isn't on it.
pub async fn delete_entry(token: &str, media_id: i32) -> Result<(), AnilistError> {
    let data = graphql(
        "query ($id: Int) { Media(id: $id) { mediaListEntry { id } } }",
        json!({ "id": media_id }),
        Some(token),
    )
    .await?;
    let Some(entry_id) = data["Media"]["mediaListEntry"]["id"].as_i64() else {
        return Ok(());
    };
    graphql(
        "mutation ($id: Int) { DeleteMediaListEntry(id: $id) { deleted } }",
        json!({ "id": entry_id }),
        Some(token),
    )
    .await?;
    Ok(())
}

pub fn authorize_url(state: &str) -> Result<String, AnilistError> {
    let config = config()?;
    let mut url = reqwest::Url::parse(AUTHORIZE_URL).expect("valid AniList authorize url");
    url.query_pairs_mut()
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", &config.redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("state", state);
    Ok(url.to_string())
}

/// Trades the code AniList redirected back with for an access token and its expiry.
pub async fn exchange_code(code: &str) -> Result<(String, NaiveDateTime), AnilistError> {
    let config = config()?;
    throttle().await;
    let response = CLIENT
        .post(TOKEN_URL)
        .header("Accept", "application/json")
        .json(&json!({
            "grant_type": "authorization_code",
            "client_id": config.client_id,
            "client_secret": config.client_secret,
            "redirect_uri": config.redirect_uri,
            "code": code,
        }))
        .send()
        .await?;
    let status = response.status();
    let body: Value = response.json().await?;
    let Some(token) = body["access_token"].as_str() else {
        let message = body["message"]
            .as_str()
            .or(body["error"].as_str())
            .unwrap_or("no access token");
        return Err(AnilistError::Other(format!("token exchange failed ({status}): {message}")));
    };
    let expires_in = body["expires_in"].as_i64().unwrap_or(365 * 24 * 60 * 60);
    let expires_at = Utc::now().naive_utc() + chrono::Duration::seconds(expires_in);
    Ok((token.to_string(), expires_at))
}

/// Where the callback sends the user once they're connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnTarget {
    App,
    Web,
}

impl ReturnTarget {
    fn code(self) -> &'static str {
        match self {
            ReturnTarget::App => "app",
            ReturnTarget::Web => "web",
        }
    }

    fn parse(code: &str) -> Option<Self> {
        match code {
            "app" => Some(ReturnTarget::App),
            "web" => Some(ReturnTarget::Web),
            _ => None,
        }
    }
}

fn state_mac(payload: &str) -> hmac::Hmac<sha2::Sha256> {
    let mut mac = SECRET_KEY.clone();
    mac.update(STATE_PREFIX);
    mac.update(payload.as_bytes());
    mac
}

/// The OAuth `state`: who is connecting and where to send them after, signed so the
/// callback can trust it without a session.
pub fn sign_state(user_id: i32, target: ReturnTarget) -> String {
    sign_state_at(user_id, target, Utc::now().timestamp() + STATE_TTL_SECS)
}

fn sign_state_at(user_id: i32, target: ReturnTarget, exp: i64) -> String {
    let payload = format!("{user_id}.{exp}.{}", target.code());
    let sig = hex::encode(state_mac(&payload).finalize().into_bytes());
    format!("{payload}.{sig}")
}

pub fn verify_state(state: &str) -> Option<(i32, ReturnTarget)> {
    let (payload, sig) = state.rsplit_once('.')?;
    let mut parts = payload.split('.');
    let user_id = parts.next()?.parse().ok()?;
    let exp: i64 = parts.next()?.parse().ok()?;
    let target = ReturnTarget::parse(parts.next()?)?;
    if parts.next().is_some() || exp < Utc::now().timestamp() {
        return None;
    }
    state_mac(payload).verify_slice(&hex::decode(sig).ok()?).ok()?;
    Some((user_id, target))
}

pub async fn connect(db: &DatabaseConnection, user_id: i32, code: &str) -> Result<String, AnilistError> {
    let (token, expires_at) = exchange_code(code).await?;
    let (anilist_user_id, name) = viewer(&token).await?;
    entity::user::ActiveModel {
        id: Set(user_id),
        anilist_token: Set(Some(token)),
        anilist_token_expires_at: Set(Some(expires_at)),
        anilist_user_id: Set(Some(anilist_user_id)),
        anilist_user_name: Set(Some(name.clone())),
        ..Default::default()
    }
    .update(db)
    .await
    .map_err(|e| AnilistError::Other(e.to_string()))?;
    info!("[AniList] User {user_id} connected AniList account {name} ({anilist_user_id})");
    Ok(name)
}

pub async fn disconnect(db: &DatabaseConnection, user_id: i32) -> Result<(), sea_orm::DbErr> {
    entity::user::ActiveModel {
        id: Set(user_id),
        anilist_token: Set(None),
        anilist_token_expires_at: Set(None),
        anilist_user_id: Set(None),
        anilist_user_name: Set(None),
        ..Default::default()
    }
    .update(db)
    .await?;
    Ok(())
}

/// The user's token, while it hasn't expired.
pub fn usable_token(user: &entity::user::Model) -> Option<&str> {
    let token = user.anilist_token.as_deref()?;
    match user.anilist_token_expires_at {
        Some(expires_at) if expires_at <= Utc::now().naive_utc() => None,
        _ => Some(token),
    }
}

/// AniList descriptions carry some HTML even with `asHtml: false`.
fn clean_description(description: &str) -> String {
    lazy_static! {
        static ref BR: regex::Regex = regex::Regex::new(r"(?i)<br\s*/?>").unwrap();
        static ref TAG: regex::Regex = regex::Regex::new(r"<[^>]+>").unwrap();
        static ref BLANK_LINES: regex::Regex = regex::Regex::new(r"\n{3,}").unwrap();
    }
    let text = BR.replace_all(description, "\n");
    let text = TAG.replace_all(&text, "");
    BLANK_LINES.replace_all(text.trim(), "\n\n").to_string()
}

/// Caches `media`'s details on the manga.
pub async fn store_media(db: &DatabaseConnection, manga_id: i32, media: &Media) -> Result<(), sea_orm::DbErr> {
    entity::manga::ActiveModel {
        id: Set(manga_id),
        anilist_title: Set(media.display_title()),
        anilist_description: Set(media.description.as_deref().map(clean_description)),
        anilist_cover: Set(media.cover()),
        anilist_banner: Set(media.banner_image.clone()),
        anilist_score: Set(media.average_score),
        anilist_genres: Set(Some(media.genres.clone())),
        anilist_status: Set(media.status.clone()),
        anilist_fetched_at: Set(Some(Utc::now().naive_utc())),
        ..Default::default()
    }
    .update(db)
    .await?;
    Ok(())
}

/// Points a manga at an AniList entry (or none) and caches its details.
pub async fn set_match(
    db: &DatabaseConnection,
    manga_id: i32,
    media: Option<&Media>,
    how: entity::manga::AnilistMatch,
) -> Result<(), sea_orm::DbErr> {
    let manga = entity::manga::ActiveModel {
        id: Set(manga_id),
        anilist_id: Set(media.map(|m| m.id)),
        anilist_match: Set(Some(how)),
        anilist_matched_at: Set(Some(Utc::now().naive_utc())),
        ..Default::default()
    };
    if media.is_none() {
        entity::manga::ActiveModel {
            anilist_title: Set(None),
            anilist_description: Set(None),
            anilist_cover: Set(None),
            anilist_banner: Set(None),
            anilist_score: Set(None),
            anilist_genres: Set(None),
            anilist_status: Set(None),
            anilist_fetched_at: Set(None),
            ..manga
        }
        .update(db)
        .await?;
        return Ok(());
    }
    manga.update(db).await?;
    if let Some(media) = media {
        store_media(db, manga_id, media).await?;
    }
    Ok(())
}

pub async fn find_user(db: &DatabaseConnection, user_id: i32) -> Result<Option<entity::user::Model>, sea_orm::DbErr> {
    entity::user::Entity::find_by_id(user_id).one(db).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_secret() {
        if std::env::var("JWT_SECRET").is_err() {
            std::env::set_var("JWT_SECRET", "test-secret");
        }
    }

    #[test]
    fn state_round_trips() {
        init_secret();
        let state = sign_state(42, ReturnTarget::Web);
        assert_eq!(verify_state(&state), Some((42, ReturnTarget::Web)));
    }

    #[test]
    fn expired_state_is_rejected() {
        init_secret();
        let state = sign_state_at(42, ReturnTarget::App, Utc::now().timestamp() - 1);
        assert_eq!(verify_state(&state), None);
    }

    #[test]
    fn tampered_state_is_rejected() {
        init_secret();
        let state = sign_state(42, ReturnTarget::App);
        let tampered = state.replacen("42.", "43.", 1);
        assert_eq!(verify_state(&tampered), None);
        assert_eq!(verify_state("garbage"), None);
    }

    #[test]
    fn description_html_is_stripped() {
        assert_eq!(
            clean_description("A <i>story</i>.<br><br>\n<br>Second<br/>line"),
            "A story.\n\nSecond\nline"
        );
    }
}
