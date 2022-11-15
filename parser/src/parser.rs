use crate::{model::*, plugin::plugins};
use anyhow::anyhow;
use async_trait::async_trait;
use futures::future::join_all;

#[async_trait]
pub trait Parser {
    async fn manga(&self, url: reqwest::Url) -> anyhow::Result<Manga>;
    async fn images(&self, url: reqwest::Url) -> anyhow::Result<Vec<reqwest::Url>>;
    async fn search(
        &self,
        keyword: String,
        hostnames: Vec<String>,
    ) -> anyhow::Result<Vec<SearchManga>>;
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
        let hostname = url.host_str().ok_or(anyhow!("No hostname in url"))?;

        let parser = self
            .parsers
            .iter()
            .find(|parser| parser.hostnames().contains(&hostname))
            .ok_or(anyhow!("No parser found for {}", hostname))?;

        parser.manga(url).await
    }
    async fn images(&self, url: reqwest::Url) -> anyhow::Result<Vec<reqwest::Url>> {
        let hostname = url.host_str().ok_or(anyhow!("No hostname in url"))?;

        let parser = self
            .parsers
            .iter()
            .find(|parser| parser.hostnames().contains(&hostname))
            .ok_or(anyhow!("No parser found for {}", hostname))?;

        parser.images(url).await
    }
    async fn search(
        &self,
        keyword: String,
        hostnames: Vec<String>,
    ) -> anyhow::Result<Vec<SearchManga>> {
        let parsers = self.parsers.iter().filter(|parser| {
            parser.can_search()
                && parser
                    .hostnames()
                    .iter()
                    .any(|hn| hostnames.contains(&hn.to_string()))
        });

        let mut processes = vec![];
        for parser in parsers {
            let supported_hostnames: Vec<String> = parser
                .hostnames()
                .into_iter()
                .filter(|hn| hostnames.contains(&hn.to_string()))
                .map(|hn| hn.to_string())
                .collect();
            processes.push(parser.clone().search(keyword.clone(), supported_hostnames));
        }
        let results: Vec<SearchManga> = join_all(processes)
            .await
            .into_iter()
            .filter(|res| res.is_ok())
            .flat_map(|results| results.unwrap())
            .collect();

        Ok(results)
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
