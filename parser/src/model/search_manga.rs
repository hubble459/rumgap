use chrono::{DateTime, Utc};

#[derive(Debug)]
pub struct SearchManga {
    pub url: reqwest::Url,
    pub title: String,
    pub updated: Option<DateTime<Utc>>,
    pub cover: Option<reqwest::Url>,
}