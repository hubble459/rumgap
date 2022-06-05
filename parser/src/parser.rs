use crate::{model::*, plugin::plugins};
use async_trait::async_trait;
use anyhow::anyhow;

#[async_trait]
pub trait Parser {
    async fn manga(&self, url: reqwest::Url) -> anyhow::Result<Manga>;
    async fn chapters(&self, url: reqwest::Url) -> anyhow::Result<Vec<Chapter>>;
    async fn images(&self, url: reqwest::Url) -> anyhow::Result<Vec<reqwest::Url>>;
    async fn search(&self, keyword: reqwest::Url) -> anyhow::Result<Vec<Manga>>;
    fn hostnames(&self) -> Vec<&'static str>;
    fn can_search(&self) -> bool;
    fn rate_limit(&self) -> u32;
}

pub struct MangaParser {
    pub parsers: Vec<Box<dyn Parser + Send + Sync>>,
}

impl MangaParser {
    pub fn new() -> MangaParser {
        MangaParser { parsers: plugins() }
    }
}

#[async_trait]
impl Parser for MangaParser {
    async fn manga(&self, url: reqwest::Url) -> anyhow::Result<Manga> {
        let hostname = url
            .host_str()
            .ok_or(anyhow!("No hostname in url"))?;

        let parser = self
            .parsers
            .iter()
            .find(|parser| parser.hostnames().contains(&hostname))
            .ok_or(anyhow!("No parser found for {}", hostname))?;

        parser.manga(url).await
    }
    async fn chapters(&self, url: reqwest::Url) -> anyhow::Result<Vec<Chapter>> {
        todo!()
    }
    async fn images(&self, url: reqwest::Url) -> anyhow::Result<Vec<reqwest::Url>> {
        todo!()
    }
    async fn search(&self, keyword: reqwest::Url) -> anyhow::Result<Vec<Manga>> {
        todo!()
    }
    fn hostnames(&self) -> Vec<&'static str> {
        self.parsers
            .iter()
            .flat_map(|parser| parser.hostnames())
            .collect()
    }

    fn can_search(&self) -> bool {
        true
    }

    fn rate_limit(&self) -> u32 {
        0
    }
}
