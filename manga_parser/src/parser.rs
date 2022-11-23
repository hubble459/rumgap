use crate::{
    model::*,
    parse_error::{ParseError, Result},
    plugin::plugins,
    util,
};
use async_trait::async_trait;
use futures::future::join_all;
use reqwest::Url;

#[async_trait]
pub trait Parser {
    async fn manga(&self, url: Url) -> Result<Manga>;
    async fn images(&self, url: &Url) -> Result<Vec<Url>>;
    async fn search(&self, keyword: String, hostnames: Vec<String>) -> Result<Vec<SearchManga>>;
    fn hostnames(&self) -> Vec<&'static str>;
    fn can_search(&self) -> Option<Vec<String>>;
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
    async fn manga(&self, url: Url) -> Result<Manga> {
        let hostname = util::get_hostname(&url)?;

        let parser = self
            .parsers
            .iter()
            .find(|parser| parser.hostnames().contains(&hostname.as_str()))
            .ok_or(ParseError::NoParserFound(hostname.to_string()))?;

        parser.manga(url).await
    }
    async fn images(&self, url: &Url) -> Result<Vec<Url>> {
        let hostname = util::get_hostname(&url)?;

        let parser = self
            .parsers
            .iter()
            .find(|parser| parser.hostnames().contains(&hostname.as_str()))
            .ok_or(ParseError::NoParserFound(hostname.to_string()))?;

        parser.images(url).await
    }
    async fn search(&self, keyword: String, hostnames: Vec<String>) -> Result<Vec<SearchManga>> {
        let parsers = self.parsers.iter().filter(|parser| {
            parser.can_search().map_or_else(
                || false,
                |arr| arr.iter().any(|hn| hostnames.contains(&hn.to_string())),
            )
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
            .filter(|res| {
                if let Err(e) = res {
                    error!("{:#?}", e);
                    false
                } else {
                    true
                }
            })
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
    fn can_search(&self) -> Option<Vec<String>> {
        let hostnames: Vec<String> = self
            .parsers
            .iter()
            .map(|parser| parser.can_search())
            .filter(|option| option.is_some())
            .flat_map(|option| option.unwrap())
            .collect();

        if hostnames.is_empty() {
            None
        } else {
            Some(hostnames)
        }
    }
    fn rate_limit(&self) -> u32 {
        0
    }
}
