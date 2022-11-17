use crate::{
    model::{Chapter, GenericQuery, Manga, SearchManga},
    parser::Parser,
    util,
};
use anyhow::{anyhow, bail, Result};
use crabquery::{Document, Element, Elements};
use futures::future::BoxFuture;
use once_cell::sync::Lazy;
use regex::{Regex, RegexBuilder};
use reqwest::Url;

pub type DocLoc = (Document, Url);

#[async_trait::async_trait]
pub trait IGenericQueryParser {
    fn get_document(&self, html: &str) -> Result<Document>;
    async fn get_document_from_url(&self, url: Url) -> Result<(String, DocLoc)>;
    async fn chapters(&self, html: &str, url: Url, title: &str) -> Result<Vec<Chapter>>;
    async fn get_images(&self, url: Url) -> Result<Vec<Url>>;
    fn abs_url(&self, location: Url, element: &Element, attrs: Vec<&'static str>) -> Result<Url>;
    fn accepts(&self, url: &Url) -> Result<()>;
    fn title(&self, doc_loc: &DocLoc) -> Result<String>;
    fn description(&self, doc_loc: &DocLoc) -> String;
    fn cover(&self, doc_loc: &DocLoc) -> Option<Url>;
    fn ongoing(&self, doc_loc: &DocLoc) -> bool;
    fn genres(&self, doc_loc: &DocLoc) -> Vec<String>;
    fn alt_titles(&self, doc_loc: &DocLoc) -> Vec<String>;
    fn authors(&self, doc_loc: &DocLoc) -> Vec<String>;
}

static GENERIC_LIST_SPLITTER: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\s\n\r\t:;\-]*").unwrap());

pub struct DefaultGenericQueryParser;

impl DefaultGenericQueryParser {
    fn collect_list(doc: &Document, query: Option<&str>, attr: Option<&str>) -> Vec<String> {
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

    fn get_document(html: &str) -> Result<Document> {
        std::panic::catch_unwind(|| {
            return Document::from(html);
        })
        .map_err(|e| anyhow!(e.downcast::<&str>().unwrap()))
    }

    async fn get_document_from_url(url: Url) -> Result<(String, DocLoc)> {
        let response = reqwest::get(url.clone()).await?;
        let url = response.url().clone();
        let html = response.text().await?;
        let document = DefaultGenericQueryParser::get_document(&html)?;
        Ok((html.to_owned(), (document, url)))
    }

    async fn images(query: &GenericQuery, url: Url) -> Result<Vec<Url>> {
        let (_, (doc, location)) = DefaultGenericQueryParser::get_document_from_url(url)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

        let images = doc.select(query.images.image);
        let mut attrs = query.images.image_attrs.clone().unwrap_or(vec![]);
        let mut default_attrs = vec!["src", "data-src"];
        attrs.append(&mut default_attrs);

        let images = images
            .into_iter()
            .map(|img| DefaultGenericQueryParser::abs_url(location.clone(), &img, attrs.to_vec()))
            .collect::<Result<Vec<Url>>>()?;

        Ok(images)
    }

    async fn chapters(
        query: &GenericQuery,
        html: String,
        _url: Url,
        manga_title: String,
    ) -> Result<Vec<Chapter>> {
        let query = &query.manga.chapter;
        let doc = DefaultGenericQueryParser::get_document(&html)?;

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

    fn abs_url(location: Url, element: &Element, attrs: Vec<&'static str>) -> Result<Url> {
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
    fn accepts(query: &GenericQuery, url: &Url) -> Result<()> {
        if let Some(hostname) = url.host_str() {
            if query.hostnames.contains(&hostname) {
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
    fn title(query: &GenericQuery, (doc, _): &DocLoc) -> Result<String> {
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
    fn description(query: &GenericQuery, (doc, _): &DocLoc) -> String {
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
    fn cover(query: &GenericQuery, (doc, _): &DocLoc) -> Option<Url> {
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
    fn ongoing(query: &GenericQuery, (doc, _): &DocLoc) -> bool {
        if let Some(ongoing_query) = query.manga.is_ongoing {
            let elements: Elements = doc.select(ongoing_query).into();

            if let Some(attr) = query.manga.ongoing_attr {
                if let Some(ongoing) = elements.attr(attr) {
                    return util::string_to_status(&ongoing);
                }
            }
        }
        true
    }
    fn genres(query: &GenericQuery, (doc, _): &DocLoc) -> Vec<String> {
        DefaultGenericQueryParser::collect_list(doc, query.manga.genres, query.manga.genres_attr)
    }
    fn alt_titles(query: &GenericQuery, (doc, _): &DocLoc) -> Vec<String> {
        DefaultGenericQueryParser::collect_list(
            doc,
            query.manga.alt_titles,
            query.manga.alt_titles_attr,
        )
    }
    fn authors(query: &GenericQuery, (doc, _): &DocLoc) -> Vec<String> {
        DefaultGenericQueryParser::collect_list(doc, query.manga.authors, query.manga.authors_attr)
    }
}

pub struct GenericQueryParserFunctions {
    pub collect_list: fn(doc: &Document, query: Option<&str>, attr: Option<&str>) -> Vec<String>,
    pub get_document: fn(&'_ str) -> Result<Document>,
    pub get_document_from_url: fn(url: Url) -> BoxFuture<'static, Result<(String, DocLoc)>>,
    pub images: fn(query: &GenericQuery, url: Url) -> BoxFuture<'_, Result<Vec<Url>>>,
    pub chapters: fn(
        query: &GenericQuery,
        html: String,
        url: Url,
        manga_title: String,
    ) -> BoxFuture<'_, Result<Vec<Chapter>>>,
    pub abs_url: fn(location: Url, element: &Element, attrs: Vec<&'static str>) -> Result<Url>,
    pub accepts: fn(query: &GenericQuery, url: &Url) -> Result<()>,
    pub title: fn(query: &GenericQuery, docLoc: &DocLoc) -> Result<String>,
    pub description: fn(query: &GenericQuery, docLoc: &DocLoc) -> String,
    pub cover: fn(query: &GenericQuery, docLoc: &DocLoc) -> Option<Url>,
    pub ongoing: fn(query: &GenericQuery, docLoc: &DocLoc) -> bool,
    pub genres: fn(query: &GenericQuery, docLoc: &DocLoc) -> Vec<String>,
    pub alt_titles: fn(query: &GenericQuery, docLoc: &DocLoc) -> Vec<String>,
    pub authors: fn(query: &GenericQuery, docLoc: &DocLoc) -> Vec<String>,
}

const DEFAULT_FUNCTIONS: GenericQueryParserFunctions = GenericQueryParserFunctions {
    get_document_from_url: |url| Box::pin(DefaultGenericQueryParser::get_document_from_url(url)),
    images: |query, url| Box::pin(DefaultGenericQueryParser::images(query, url)),
    chapters: |query, html, url, manga_title| {
        Box::pin(DefaultGenericQueryParser::chapters(
            query,
            html,
            url,
            manga_title,
        ))
    },
    collect_list: DefaultGenericQueryParser::collect_list,
    get_document: DefaultGenericQueryParser::get_document,
    abs_url: DefaultGenericQueryParser::abs_url,
    accepts: DefaultGenericQueryParser::accepts,
    title: DefaultGenericQueryParser::title,
    description: DefaultGenericQueryParser::description,
    cover: DefaultGenericQueryParser::cover,
    ongoing: DefaultGenericQueryParser::ongoing,
    genres: DefaultGenericQueryParser::genres,
    alt_titles: DefaultGenericQueryParser::alt_titles,
    authors: DefaultGenericQueryParser::authors,
};

pub struct GenericQueryParser {
    pub query: GenericQuery,
    pub functions: GenericQueryParserFunctions,
}

impl GenericQueryParser {
    pub fn new(generic_query: GenericQuery) -> Self {
        Self {
            query: generic_query,
            functions: DEFAULT_FUNCTIONS,
        }
    }
}

#[async_trait::async_trait]
impl IGenericQueryParser for GenericQueryParser {
    fn get_document(&self, html: &str) -> Result<Document> {
        (self.functions.get_document)(html)
    }

    async fn get_document_from_url(&self, url: Url) -> Result<(String, DocLoc)> {
        (self.functions.get_document_from_url)(url).await
    }

    async fn chapters(&self, html: &str, url: Url, title: &str) -> Result<Vec<Chapter>> {
        (self.functions.chapters)(&self.query, html.to_owned(), url, title.to_owned()).await
    }

    async fn get_images(&self, url: Url) -> Result<Vec<Url>> {
        (self.functions.images)(&self.query, url).await
    }

    fn abs_url(&self, location: Url, element: &Element, attrs: Vec<&'static str>) -> Result<Url> {
        (self.functions.abs_url)(location, element, attrs)
    }

    fn accepts(&self, url: &Url) -> Result<()> {
        (self.functions.accepts)(&self.query, url)
    }

    fn title(&self, doc_loc: &DocLoc) -> Result<String> {
        (self.functions.title)(&self.query, doc_loc)
    }

    fn description(&self, doc_loc: &DocLoc) -> String {
        (self.functions.description)(&self.query, doc_loc)
    }

    fn cover(&self, doc_loc: &DocLoc) -> Option<Url> {
        (self.functions.cover)(&self.query, doc_loc)
    }

    fn ongoing(&self, doc_loc: &DocLoc) -> bool {
        (self.functions.ongoing)(&self.query, doc_loc)
    }

    fn genres(&self, doc_loc: &DocLoc) -> Vec<String> {
        (self.functions.genres)(&self.query, doc_loc)
    }

    fn alt_titles(&self, doc_loc: &DocLoc) -> Vec<String> {
        (self.functions.alt_titles)(&self.query, doc_loc)
    }

    fn authors(&self, doc_loc: &DocLoc) -> Vec<String> {
        (self.functions.authors)(&self.query, doc_loc)
    }
}

#[async_trait::async_trait]
impl Parser for GenericQueryParser {
    async fn manga(&self, url: Url) -> Result<Manga> {
        self.accepts(&url)?;

        let result: Result<_> = {
            let (html, doc_loc) = self
                .get_document_from_url(url)
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

        manga.chapters = self
            .chapters(&html, manga.url.clone(), &manga.title)
            .await?;

        Ok(manga)
    }

    async fn images(&self, url: Url) -> Result<Vec<Url>> {
        (self.functions.images)(&self.query, url).await
    }

    async fn search(&self, keyword: String, hostnames: Vec<String>) -> Result<Vec<SearchManga>> {
        let mut results = vec![];

        let mut searchable_hostnames = self.hostnames();

        if let Some(search) = &self.query.search {
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
    fn hostnames(&self) -> Vec<&'static str> {
        self.query.hostnames.clone()
    }
    fn can_search(&self) -> bool {
        self.query.search.is_some()
    }
    fn rate_limit(&self) -> u32 {
        100
    }
}
