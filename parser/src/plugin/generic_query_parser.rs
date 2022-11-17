use std::time::Duration;

use crate::{
    model::{Chapter, GenericQuery, GenericQuerySearch, Manga, SearchManga},
    parser::Parser,
    util,
};
use anyhow::{anyhow, bail, Result};
use cloudflare_bypasser;
use crabquery::{Document, Element, Elements};
use once_cell::sync::Lazy;
use regex::{Regex, RegexBuilder};
use reqwest::{Response, StatusCode, Url};

pub type DocLoc = (Document, Url);

static GENERIC_LIST_SPLITTER: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\s\n\r\t:;\-]*").unwrap());

#[async_trait::async_trait]
pub trait IGenericQueryParser: Parser {
    fn new(query: GenericQuery) -> Self
    where
        Self: Sized + Send + Sync;

    fn collect_list(&self, doc: &Document, query: Option<&str>, attr: Option<&str>) -> Vec<String> {
        if let Some(query) = query {
            let elements: Elements = doc.select(query).into();
            if let Some(attr) = attr {
                if let Some(text) = elements.attr(attr) {
                    return text.split("\n").map(String::from).collect();
                }
            } else {
                if let Some(text) = elements.text() {
                    if elements.elements.len() > 1 {
                        return text.split("\n").map(String::from).collect();
                    } else {
                        return GENERIC_LIST_SPLITTER
                            .split(&text)
                            .map(String::from)
                            .collect();
                    }
                }
            }
        }
        return vec![];
    }
    fn get_query(&self) -> &GenericQuery;
    fn get_document(&self, html: &str) -> Result<Document> {
        std::panic::catch_unwind(|| {
            return Document::from(html);
        })
        .map_err(|e| anyhow!(e.downcast::<&str>().unwrap()))
    }
    async fn request(
        &self,
        url: &Url,
        user_agent: &str,
        cookie: &str,
    ) -> Result<Response> {
        let response = reqwest::Client::new()
            .get(url.clone())
            .header(reqwest::header::USER_AGENT, user_agent)
            .header(reqwest::header::COOKIE, cookie)
            .header("Accept", "*/*")
            .header("Referer", url.to_string())
            .timeout(Duration::from_secs(5))
            .send()
            .await?;

        if response.status() == StatusCode::FORBIDDEN {
            let mut bypasser = cloudflare_bypasser::Bypasser::default()
                .retry(10)
                .random_user_agent(true);
            let mut retries = 0;
            loop {
                if let Ok((c, ua)) = bypasser.bypass(&url.to_string()) {
                    return Ok(self.request(url, ua.to_str().unwrap(), c.to_str().unwrap()).await?);
                } else if retries == 10 {
                    break;
                } else {
                    retries += 1;
                }
            }
            bail!("Cloudflare timeout");
        }

        Ok(response)
    }
    async fn get_document_from_url(&self, url: &Url) -> Result<(String, DocLoc)> {
        let response = self
            .request(
                url,
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:107.0) Gecko/20100101 Firefox/107.0",
                "",
            )
            .await?;

        let url = response.url().clone();
        let html = response.text().await?;
        let document = self.get_document(&html)?;
        Ok((html, (document, url)))
    }
    async fn chapters(&self, html: &str, url: &Url, manga_title: &str) -> Result<Vec<Chapter>> {
        let query = &self.get_query().manga.chapter;
        let doc = self.get_document(&html)?;

        let elements: Elements = doc.select(query.base).into();

        let mut chapters = vec![];

        let href_attrs =
            util::merge_attr_with_default(&query.href_attr, vec!["href", "src", "data-src"]);

        let mut chapter_number_fallback = elements.elements.len();

        for element in elements.elements.iter() {
            // Href
            let href = element.select(query.href);
            let href = href.first().ok_or(anyhow!("Missing href for a chapter"))?;
            let url = self.abs_url(url, href, &href_attrs)?;

            // Title
            let title_element = if let Some(title_query) = query.title {
                if title_query == query.href {
                    href.clone()
                } else {
                    let title = element.select(title_query);
                    let title = title.first().cloned();
                    title.ok_or(anyhow!("Missing title for a chapter"))?
                }
            } else {
                href.clone()
            };
            let title = if let Some(title_attr) = query.title_attr {
                title_element.attr(title_attr)
            } else {
                title_element.text()
            };
            let title = title.ok_or(anyhow!("Missing title for a chapter"))?;
            // Remove manga title from chapter title
            let title = RegexBuilder::new(&("$".to_owned() + &manga_title.to_lowercase()))
                .case_insensitive(true)
                .build()
                .unwrap()
                .replace(&title, "")
                .to_string();

            // Number (is in title or we get fallback)
            let number_element = if let Some(number_query) = query.number {
                let element = element.select(number_query);
                element.first().cloned().unwrap_or(title_element)
            } else {
                title_element
            };
            let number = if let Some(attr) = query.number_attr {
                number_element.attr(attr)
            } else {
                number_element.text()
            };
            let number: f32 = if let Some(number) = number {
                let number = Regex::new(r"\d+").unwrap().find_iter(&number).last();
                if let Some(number) = number {
                    number.as_str().parse().unwrap()
                } else {
                    chapter_number_fallback as f32
                }
            } else {
                chapter_number_fallback as f32
            };

            chapters.push(Chapter {
                url,
                title,
                number,
                posted: None,
            });

            chapter_number_fallback -= 1;
        }

        Ok(chapters)
    }
    async fn get_manga(&self, url: Url) -> Result<Manga> {
        self.accepts(&url)?;

        let result: Result<_> = {
            let (html, doc_loc) = self
                .get_document_from_url(&url)
                .await
                .map_err(|e| anyhow!(e.to_string()))?;

            Ok((
                html.to_owned(),
                Manga {
                    url: doc_loc.1.clone(),
                    title: self.title(&doc_loc)?,
                    description: self.description(&doc_loc),
                    cover: self.cover(&doc_loc),
                    ongoing: self.ongoing(&doc_loc),
                    genres: self.genres(&doc_loc),
                    authors: self.authors(&doc_loc),
                    alt_titles: self.alt_titles(&doc_loc),
                    chapters: vec![],
                },
            ))
        };

        let (html, mut manga) = result?;

        manga.chapters = self.chapters(&html, &manga.url, &manga.title).await?;

        Ok(manga)
    }
    async fn get_images(&self, url: &Url) -> Result<Vec<Url>> {
        let query = self.get_query();
        let (_, (doc, location)) = self
            .get_document_from_url(url)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

        let images = doc.select(query.images.image);
        let mut attrs = query.images.image_attrs.clone().unwrap_or(vec![]);
        let mut default_attrs = vec!["src", "data-src"];
        attrs.append(&mut default_attrs);

        let images = images
            .into_iter()
            .map(|img| self.abs_url(&location, &img, &attrs.to_vec()))
            .collect::<Result<Vec<Url>>>()?;

        Ok(images)
    }
    fn parse_keywords(&self, keywords: &str) -> String {
        if let Some(GenericQuerySearch { encode, .. }) = self.get_query().search {
            if encode {
                return urlencoding::encode(keywords).into_owned();
            }
        }
        keywords.to_owned()
    }
    fn parse_search_url(&self, hostname: &str, keywords: &str, path: &str) -> Result<Url> {
        let path = path.trim_start_matches("/");
        let path = path.replace("[query]", &self.parse_keywords(keywords));
        let url = format!("https://{}/{}", hostname, path);
        let url = Url::parse(&url)?;

        Ok(url)
    }
    async fn do_search(
        &self,
        keywords: String,
        hostnames: Vec<String>,
    ) -> Result<Vec<SearchManga>> {
        let query = self.get_query();
        if let Some(query) = &query.search {
            let mut searchable_hostnames = self.hostnames();

            if let Some(hostnames) = &query.hostnames {
                searchable_hostnames = hostnames.clone();
            }

            if !searchable_hostnames.is_empty() {
                let mut results = vec![];
                let path = query.path;
                for hostname in hostnames.iter() {
                    // If hostname is searchable
                    if searchable_hostnames.contains(&hostname.as_str()) {
                        let url = self.parse_search_url(&hostname, &keywords, &path)?;
                        // Add results to results array
                        let result = self.get_document_from_url(&url).await;
                        let doc = match result {
                            Err(e) => {
                                error!("{:#?}", e);
                                continue;
                            }
                            Ok((_, (doc, _))) => doc,
                        };

                        let elements = doc.select(query.base);

                        let href_attrs = util::merge_attr_with_default(
                            &query.href_attr,
                            vec!["href", "src", "data-src"],
                        );

                        for element in elements {
                            // Href
                            let href = element.select(query.href);
                            let href = href
                                .first()
                                .ok_or(anyhow!("Missing href for a search item"))?;
                            let url = util::first_attr(href, &href_attrs)
                                .ok_or(anyhow!("Missing href for a search item"))?;
                            let url = Url::parse(&url).map_err(|e| anyhow!(e))?;

                            // Title
                            let title_element = if let Some(title_query) = query.title {
                                if title_query == query.href {
                                    href.clone()
                                } else {
                                    let title = element.select(title_query);
                                    let title = title.first().cloned();
                                    title.ok_or(anyhow!("Missing title for a search item"))?
                                }
                            } else {
                                href.clone()
                            };
                            let title = if let Some(title_attr) = query.title_attr {
                                title_element.attr(title_attr)
                            } else {
                                title_element.text()
                            };
                            let title = title.ok_or(anyhow!("Missing title for a search item"))?;

                            results.push(SearchManga {
                                url,
                                title,
                                updated: None,
                                cover: None,
                            });
                        }
                    }
                }
                Ok(results)
            } else {
                bail!("Missing hostnames to search on")
            }
        } else {
            bail!("Tried to search on a parser that does not support this feature")
        }
    }
    fn abs_url(&self, location: &Url, element: &Element, attrs: &Vec<&'static str>) -> Result<Url> {
        for attr in attrs.iter() {
            let url = &element.attr(attr);
            if let Some(url) = url {
                let result = Url::parse(&url.to_string());
                if let Err(_) = result {
                    let mut base = location.clone();
                    base.set_path("/");
                    let url = base.join(&url.to_string())?;
                    return Ok(url);
                } else {
                    return Ok(result?);
                }
            }
        }
        bail!("No url found in element (with attrs: {:?})", attrs);
    }
    fn accepts(&self, url: &Url) -> Result<()> {
        if let Some(hostname) = url.host_str() {
            if self.get_query().hostnames.contains(&hostname) {
                return Ok(());
            } else {
                bail!(
                    "This parser does not support this url \"{:?}\"",
                    url.to_string()
                );
            }
        } else {
            bail!("Url should have a hostname");
        }
    }
    fn title(&self, (doc, _): &DocLoc) -> Result<String> {
        let query = self.get_query();
        let elements: Elements = doc.select(&query.manga.title).into();

        if let Some(attr) = query.manga.title_attr {
            if let Some(title) = elements.attr(attr) {
                Ok(title)
            } else {
                elements.text().ok_or(anyhow!("Missing title"))
            }
        } else {
            elements.text().ok_or(anyhow!("Missing title"))
        }
    }
    fn description(&self, (doc, _): &DocLoc) -> String {
        let query = self.get_query();

        if let Some(description_query) = query.manga.description {
            let elements: Elements = doc.select(description_query).into();

            if let Some(attr) = query.manga.description_attr {
                if let Some(description) = elements.attr(attr) {
                    return description;
                }
            }
            elements.text().unwrap_or("No description".to_owned())
        } else {
            String::from("value")
        }
    }
    fn cover(&self, (doc, _): &DocLoc) -> Option<Url> {
        let query = self.get_query();

        if let Some(cover_query) = query.manga.cover {
            let elements: Elements = doc.select(cover_query).into();

            let attrs =
                util::merge_vec_with_default(&query.manga.cover_attrs, vec!["src", "data-src"]);

            if let Some(cover) = &elements.attrs(attrs) {
                if let Ok(url) = Url::parse(cover) {
                    return Some(url);
                }
            }
        }
        None
    }
    fn ongoing(&self, (doc, _): &DocLoc) -> bool {
        let query = self.get_query();

        if let Some(ongoing_query) = query.manga.is_ongoing {
            let elements: Elements = doc.select(ongoing_query).into();

            if let Some(attr) = query.manga.is_ongoing_attr {
                if let Some(ongoing) = elements.attr(attr) {
                    return util::string_to_status(&ongoing);
                }
            }
        }
        true
    }
    fn genres(&self, (doc, _): &DocLoc) -> Vec<String> {
        let query = self.get_query();

        self.collect_list(doc, query.manga.genres, query.manga.genres_attr)
    }
    fn alt_titles(&self, (doc, _): &DocLoc) -> Vec<String> {
        let query = self.get_query();

        self.collect_list(doc, query.manga.alt_titles, query.manga.alt_titles_attr)
    }
    fn authors(&self, (doc, _): &DocLoc) -> Vec<String> {
        let query = self.get_query();

        self.collect_list(doc, query.manga.authors, query.manga.authors_attr)
    }

    fn parser_can_search(&self) -> bool {
        self.get_query().search.is_some()
    }
    fn parser_hostnames(&self) -> Vec<&'static str> {
        self.get_query().hostnames.clone()
    }
    fn parser_rate_limit(&self) -> u32 {
        100
    }
}

pub struct GenericQueryParser {
    query: GenericQuery,
}

impl IGenericQueryParser for GenericQueryParser {
    fn new(query: GenericQuery) -> Self {
        Self { query }
    }

    fn get_query(&self) -> &GenericQuery {
        &self.query
    }
}

#[async_trait::async_trait]
impl Parser for GenericQueryParser {
    fn can_search(&self) -> bool {
        self.parser_can_search()
    }

    fn hostnames(&self) -> Vec<&'static str> {
        self.parser_hostnames()
    }

    fn rate_limit(&self) -> u32 {
        self.parser_rate_limit()
    }

    async fn images(&self, url: &Url) -> Result<Vec<Url>> {
        self.get_images(url).await
    }

    async fn manga(&self, url: Url) -> Result<Manga> {
        self.get_manga(url).await
    }

    async fn search(&self, keyword: String, hostnames: Vec<String>) -> Result<Vec<SearchManga>> {
        self.do_search(keyword, hostnames).await
    }
}
