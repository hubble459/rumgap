use reqwest::Url;
use serde::Deserialize;

use super::generic_query_parser::IGenericQueryParser;
use crate::{
    model::{GenericQuery, GenericQueryImages, GenericQueryManga, GenericQueryMangaChapter},
    parse_error::{ParseError, Result},
};

#[derive(parser_macro_derive::ParserDerive)]
pub struct Manga347 {
    query: GenericQuery,
}

impl Manga347 {
    pub fn new() -> Self {
        let query = GenericQuery {
            manga: GenericQueryManga {
                title: "h1.manga-name",
                description: Some("div.description"),
                cover: Some("#primaryimage"),
                cover_attrs: Some(vec!["data-src"]),
                is_ongoing: Some("span.item-head:icontains(status) + span"),
                alt_titles: Some("div.manga-name-or"),
                chapter: GenericQueryMangaChapter {
                    base: "a.item-link",
                    title: Some("span.name"),
                    posted: Some("td.episode-date"),
                    ..Default::default()
                },
                ..Default::default()
            },
            images: GenericQueryImages {
                image: "img[data-src].lazy",
                ..Default::default()
            },
            search: None,
            hostnames: vec!["manga347.com"],
            ..Default::default()
        };
        Self { query }
    }
}

#[derive(Deserialize)]
struct AjaxImageResponse {
    html: String,
}

#[async_trait::async_trait]
impl IGenericQueryParser for Manga347 {
    fn get_query(&self) -> &GenericQuery {
        &self.query
    }

    async fn images_from_url(&self, url: &Url) -> Result<Vec<Url>> {
        let id = url
            .path_segments()
            .ok_or(ParseError::InvalidChapterUrl(url.to_string()))?
            .last()
            .ok_or(ParseError::InvalidChapterUrl(url.to_string()))?;

        let url = url
            .join(&format!("/ajax/image/chapter/{}", id))
            .map_err(|_| ParseError::InvalidChapterUrl(url.to_string()))?;

        let response = self.request(&url, None).await?;
        let url = response.url().clone();
        let json: AjaxImageResponse = response.json().await?;
        let document = self.get_document(&json.html)?;

        self.get_images((document, url))
    }
}
