use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchManga {
    pub url: reqwest::Url,
    pub title: String,
    pub posted: Option<DateTime<Utc>>,
    pub cover: Option<reqwest::Url>,
}