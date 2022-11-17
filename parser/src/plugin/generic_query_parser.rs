use crate::{
    model::{Chapter, GenericQuery, Manga, SearchManga},
    parser::Parser,
    util,
};
use anyhow::{anyhow, bail, Result};
use crabquery::{Document, Element, Elements};
use once_cell::sync::Lazy;
use regex::{Regex, RegexBuilder};
use reqwest::Url;

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
    async fn get_document_from_url(&self, url: &Url) -> Result<(String, DocLoc)> {
        let response = reqwest::get(url.clone()).await?;
        let url = response.url().clone();
        let html = response.text().await?;
        let document = self.get_document(&html)?;
        Ok((html.to_owned(), (document, url)))
    }
    async fn chapters(&self, html: &str, url: &Url, manga_title: &str) -> Result<Vec<Chapter>> {
        let query = &self.get_query().manga.chapter;
        let doc = self.get_document(&html)?;

        let elements: Elements = doc.select(query.base).into();

        let mut chapters = vec![];

        let href_attrs =
            util::merge_attr_with_default(&query.href_attr, vec!["href", "src", "data-src"]);
        let title_attrs = util::merge_attr_with_default(&query.title_attr, vec![]);

        let mut chapter_number_fallback = elements.elements.len();

        for element in elements.elements.iter() {
            // Href
            let href = element.select(query.href);
            let href = href.first().ok_or(anyhow!("Missing href for a chapter"))?;
            let url =
                util::first_attr(href, &href_attrs).ok_or(anyhow!("Missing href for a chapter"))?;
            let url = Url::parse(&url).map_err(|e| anyhow!(e))?;

            // Title
            let title = if let Some(title_query) = query.title {
                let title = element.select(title_query);
                let title = title.first();
                let title = title.ok_or(anyhow!("Missing title for a chapter"))?;
                if !title_attrs.is_empty() {
                    if let Some(title_text) = util::first_attr(title, &title_attrs) {
                        Some(title_text)
                    } else {
                        title.text()
                    }
                } else {
                    title.text()
                }
            } else {
                href.text()
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
            let number = Regex::new(r"\d+").unwrap().find_iter(&title).last();
            let number = if let Some(number) = number {
                number.as_str().parse().unwrap()
            } else {
                chapter_number_fallback
            };

            chapters.push(Chapter {
                url,
                title,
                number: number as f32,
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
            .map(|img| self.abs_url(location.clone(), &img, attrs.to_vec()))
            .collect::<Result<Vec<Url>>>()?;

        Ok(images)
    }
    async fn do_search(&self, keyword: String, hostnames: Vec<String>) -> Result<Vec<SearchManga>> {
        let query = self.get_query();
        let mut results = vec![];

        let mut searchable_hostnames = self.hostnames();

        if let Some(search) = &query.search {
            if let Some(searchable) = &search.hostnames {
                searchable_hostnames = searchable.clone();
            }
        }

        if !searchable_hostnames.is_empty() {
            for hostname in hostnames.iter() {
                if searchable_hostnames.contains(&hostname.as_str()) {
                    // If hostname is searchable
                    todo!()
                    // Add results to results array
                }
            }
        }

        Ok(results)
    }
    fn abs_url(&self, location: Url, element: &Element, attrs: Vec<&'static str>) -> Result<Url> {
        for attr in attrs.iter() {
            let url = &element.attr(attr);
            if let Some(url) = url {
                let url = Url::parse(&url.to_string());
                if let Ok(mut url) = url {
                    if url.domain().is_none() {
                        url = Url::parse(&format!(
                            "{}{}",
                            location.origin().ascii_serialization(),
                            url.path()
                        ))?;
                    }
                    return Ok(url);
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
