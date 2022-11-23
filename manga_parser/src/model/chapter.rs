#[derive(Debug, Clone)]
pub struct Chapter {
    pub url: reqwest::Url,
    pub title: String,
    pub number: f32,
    pub posted: Option<chrono::DateTime<chrono::Utc>>,
}
