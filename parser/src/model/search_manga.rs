#[derive(Debug)]
pub struct SearchManga {
    pub url: reqwest::Url,
    pub title: String,
    pub updated: Option<chrono::NaiveDateTime>,
    pub cover: Option<reqwest::Url>,
}