use std::collections::HashMap;

use regex::Regex;
use reqwest::{Client, Url};
use serde::Deserialize;

use super::generic_query_parser::{GenericQueryParser, IGenericQueryParser};

use crate::{
    model::{
        Chapter, GenericQuery, GenericQueryImages, GenericQueryManga, GenericQueryMangaChapter,
    },
    parse_error::{ParseError, Result},
    util,
};

const AJAX_SPECIAL: [&str; 6] = [
    "247manga.com",
    "lhtranslation.net",
    "mangakik.com",
    "mangakik.net",
    "mangasushi.org",
    "manhuaus.com",
];

/// action=manga_get_chapters
const AJAX_NORMAL: [&str; 4] = [
    "mangafoxfull.com",
    "mangazukiteam.com",
    "mangaonlineteam.com",
    "yaoi.mobi"
];

const AJAX_IMAGES: [&str; 1] = ["azmanhwa.net"];

#[derive(parser_macro_derive::ParserDerive)]
pub struct Madara {
    query: GenericQuery,
    parser: GenericQueryParser,
}

impl Madara {
    pub fn new() -> Self {
        let query = GenericQuery {
            manga: GenericQueryManga {
                title: "div.post-title h1",
                description: Some("div.description-summary h3, div.summary__content p, div.dsct p"),
                is_ongoing: Some("div.summary-heading:has(h5:icontains(status)) + div"),
                cover: Some("div.summary_image img.lazyloaded"),
                cover_attrs: Some(vec!["data-src"]),
                authors: Some("div.author-content > a"),
                genres: Some("div.genres-content > a"),
                alt_titles: Some("div.summary-heading:has(h5:icontains(alt)) + div"),
                chapter: GenericQueryMangaChapter {
                    base: "li.wp-manga-chapter, ul.row-content-chapter li",
                    href: Some("a"),
                    posted: Some("i, span.chapter-time"),
                    ..Default::default()
                },
                ..Default::default()
            },
            images: GenericQueryImages {
                image: "div img.wp-manga-chapter-img, div.text-left > p > img, img[alt*=Page]",
                ..Default::default()
            },
            search: None,
            hostnames: vec![
                "1stkissmanga.club",
                "1stkissmanga.io",
                "1stkissmanga.com",
                "1stkissmanga.love",
                "247manga.com",
                "aquamanga.com",
                "azmanhwa.net",
                "isekaiscanmanga.com",
                "isekaiscan.com",
                "lhtranslation.net",
                "manga68.com",
                "mangaboat.com",
                "mangachill.io",
                "mangafoxfull.com",
                "mangahz.com",
                "mangaonlineteam.com",
                "mangarockteam.com",
                "mangasushi.org",
                "mangatx.com",
                "mangaweebs.in",
                "mangazukiteam.com",
                "manhuadex.com",
                "manhuaplus.com",
                "manhuaus.com",
                "manhwatop.com",
                "mixedmanga.com",
                "s2manga.com",
                "topmanhua.com",
                "yaoi.mobi",
                "zinmanga.com",
            ],
            ..Default::default()
        };
        Self {
            query: query.clone(),
            parser: GenericQueryParser::new(query),
        }
    }
}

#[async_trait::async_trait]
impl IGenericQueryParser for Madara {
    fn get_query(&self) -> &GenericQuery {
        &self.query
    }

    async fn chapters(&self, html: &str, url: &Url, manga_title: &str) -> Result<Vec<Chapter>> {
        let mut url = url.clone();
        let hostname = &url.domain().unwrap();
        let html = if AJAX_SPECIAL.contains(hostname) {
            let chapter_url = url.join("ajax/chapters/").unwrap();
            let builder = Client::default().post(chapter_url.clone());
            let response = self.request(&chapter_url, Some(builder)).await?;
            url = response.url().clone();
            response.text().await.map_err(|_| ParseError::BadHTML)?
        } else if AJAX_NORMAL.contains(hostname) {
            let id = {
                let doc = self.get_document(html)?;
                let el = util::select(&doc, "input.rating-post-id, #wp-manga-js-extra");
                let el = el.elements.first();
                if let Some(el) = el {
                    let id = el.attr("value");
                    if let None = id {
                        let script = el
                            .text()
                            .ok_or(ParseError::OtherStr("Cannot read script text"))?;
                        let regex = Regex::new(r#""manga_id":"(\d+)""#).unwrap();
                        let found = regex.captures(&script);
                        if let Some(found) = found {
                            found
                                .get(1)
                                .ok_or(ParseError::OtherStr(
                                    "Could not find manga ID for chapters",
                                ))?
                                .as_str()
                                .to_owned()
                        } else {
                            return Err(ParseError::OtherStr(
                                "Could not find manga ID for chapters",
                            ));
                        }
                    } else {
                        id.unwrap()
                    }
                } else {
                    return Err(ParseError::OtherStr("Could not find manga ID for chapters"));
                }
            };

            let chapter_url = url.join("/wp-admin/admin-ajax.php").unwrap();
            let mut params = HashMap::new();
            params.insert("action", "manga_get_chapters");
            params.insert("manga", &id);
            let builder = Client::default().post(chapter_url.clone()).form(&params);
            let response = self.request(&chapter_url, Some(builder)).await?;
            url = response.url().clone();
            response.text().await.map_err(|_| ParseError::BadHTML)?
        } else {
            html.to_owned()
        };

        self.parser.chapters(&html, &url, manga_title).await
    }

    async fn images_from_url(&self, url: &Url) -> Result<Vec<Url>> {
        let hostname = &url.domain().unwrap();

        if AJAX_IMAGES.contains(hostname) {
            let chapter_id = {
                let doc = self.get_document_from_url(url).await?.1 .0;
                let element = util::select(&doc, "script:contains(chapter_id)");
                let element = element
                    .elements
                    .first()
                    .ok_or(ParseError::OtherStr("Cannot find script with chapter id"))?;
                let script = element
                    .text()
                    .ok_or(ParseError::OtherStr("Cannot read script text"))?;
                let regex = Regex::new(r#"chapter_id\s*=\s*(\d+)"#).unwrap();
                let found = regex.captures(&script);
                if let Some(found) = found {
                    found
                        .get(1)
                        .ok_or(ParseError::OtherStr("Could not find chapter ID for images"))?
                        .as_str()
                        .to_owned()
                } else {
                    return Err(ParseError::OtherStr("Could not find chapter ID for images"));
                }
            };
            let mut url = url
                .join(&format!("/ajax/image/list/chap/{}", chapter_id))
                .map_err(|_| ParseError::InvalidChapterUrl(url.to_string()))?;
            url.set_query(Some("mode=vertical&quality=high"));

            debug!("{}", url.to_string());

            let response = self.request(&url, None).await?;
            let url = response.url().clone();
            let json: AjaxImageResponse = response.json().await?;
            let document = self.get_document(&json.html)?;

            self.get_images((document, url))
        } else {
            self.parser.images_from_url(url).await
        }
    }
}

#[derive(Deserialize)]
struct AjaxImageResponse {
    html: String,
}
