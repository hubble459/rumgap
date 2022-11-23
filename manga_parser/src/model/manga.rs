use super::chapter::Chapter;

#[derive(Debug, Clone)]
pub struct Manga {
    pub url: reqwest::Url,
    pub title: String,
    pub description: String,
    pub cover: Option<reqwest::Url>,
    pub ongoing: bool,
    pub genres: Vec<String>,
    pub authors: Vec<String>,
    pub alt_titles: Vec<String>,
    pub chapters: Vec<Chapter>,
}