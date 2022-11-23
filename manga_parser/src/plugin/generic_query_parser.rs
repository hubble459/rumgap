use std::time::Duration;

use crate::{
    model::{Chapter, GenericQuery, GenericQuerySearch, Manga, SearchManga},
    parse_error::{ParseError, Result},
    parser::Parser,
    util,
};
use crabquery::{Document, Element, Elements};
use regex::{Regex, RegexBuilder};
use reqwest::{RequestBuilder, Response, StatusCode, Url};

pub type DocLoc = (Document, Url);

lazy_static! {
    static ref GENERIC_LIST_SPLITTER: Regex = Regex::new(r"[\s\n\r\t:;\-]*").unwrap();
    static ref NORMALIZE_TITLE_MANGA: Regex = RegexBuilder::new(r"\s*manga\s*")
        .case_insensitive(true)
        .build()
        .unwrap();
}

#[async_trait::async_trait]
pub trait IGenericQueryParser: Parser {
    fn collect_list(&self, doc: &Document, query: Option<&str>, attr: Option<&str>) -> Vec<String> {
        if let Some(query) = query {
            let elements: Elements = util::select(doc, query).into();
            if let Some(attr) = attr {
                if let Some(text) = elements.attr(attr) {
                    return text.split("\n").map(|t| String::from(t.trim())).collect();
                }
            } else {
                if let Some(text) = elements.text() {
                    if elements.elements.len() > 1 {
                        return text.split("\n").map(|t| String::from(t.trim())).collect();
                    } else {
                        return GENERIC_LIST_SPLITTER
                            .split(&text)
                            .map(|t| String::from(t.trim()))
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
        .map_err(|_e| ParseError::BadHTML)
    }
    async fn request(&self, url: &Url, builder: Option<RequestBuilder>) -> Result<Response> {
        // TODO: Fix CloudFlare IUAM

        let builder = if let Some(builder) = builder {
            builder
        } else {
            reqwest::Client::new().get(url.clone())
        };

        let response = builder
            .header(
                reqwest::header::USER_AGENT,
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:107.0) Gecko/20100101 Firefox/107.0",
            )
            .header("Accept", "*/*")
            .header("Referer", url.to_string())
            .timeout(Duration::from_secs(5))
            .send()
            .await?;

        if response.status() == StatusCode::FORBIDDEN {
            return Err(ParseError::CloudflareIUAM);
        }
        if !response.status().is_success() {
            return Err(ParseError::NetworkError(response.status()));
        }

        Ok(response)
    }
    async fn get_document_from_url(&self, url: &Url) -> Result<(String, DocLoc)> {
        let response = self.request(url, None).await?;

        let url = response.url().clone();
        let html = response.text().await.map_err(|_| ParseError::BadHTML)?;
        let document = self.get_document(&html)?;
        Ok((html, (document, url)))
    }
    async fn chapters(&self, html: &str, url: &Url, manga_title: &str) -> Result<Vec<Chapter>> {
        let query = &self.get_query().manga.chapter;
        let doc = self.get_document(&html)?;

        let elements = util::select(&doc, query.base);

        let mut chapters = vec![];

        let href_attrs = util::merge_attr_with_default(&query.href_attr, vec!["href"]);

        let mut chapter_number_fallback = elements.elements.len();

        for element in elements.elements.iter() {
            // Href
            let href = util::select_element_fallback(element, query.href, Some(element.clone()));
            let href = href.ok_or(ParseError::MissingChapterHref)?;
            let url = self.abs_url(url, &href, &href_attrs)?;

            // Title
            let title_element =
                util::select_element_fallback(element, query.title, Some(href.clone()))
                    .ok_or(ParseError::MissingChapterTitle)?;
            let title = util::select_text_or_attr(title_element.clone(), query.title_attr)
                .ok_or(ParseError::MissingChapterTitle)?;
            // Remove manga title from chapter title
            let title = RegexBuilder::new(&("$".to_owned() + &manga_title.to_lowercase()))
                .case_insensitive(true)
                .build()
                .unwrap()
                .replace(&title, "")
                .trim()
                .to_string();

            // Number (is in title or we get fallback)
            let number = util::select_fallback(
                element,
                query.number,
                query.number_attr,
                Some(title_element.clone()),
            );
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

            // Posted
            let posted = util::select_fallback(
                element,
                query.posted,
                query.posted_attr,
                Some(element.clone()),
            );
            let posted = if let Some(date) = posted {
                util::try_parse_date(&date)
            } else {
                None
            };

            chapters.push(Chapter {
                url,
                title,
                number,
                posted,
            });

            chapter_number_fallback -= 1;
        }

        Ok(chapters)
    }
    async fn get_manga(&self, url: Url) -> Result<Manga> {
        self.accepts(&url)?;

        let result: Result<_> = {
            let (html, doc_loc) = self.get_document_from_url(&url).await?;

            Ok((
                html.to_owned(),
                Manga {
                    url: doc_loc.1.clone(),
                    title: self.normalize_title(self.title(&doc_loc)?.as_str()),
                    description: self.description(&doc_loc)?.trim().to_owned(),
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
    fn get_images(&self, (doc, loc): DocLoc) -> Result<Vec<Url>> {
        let query = self.get_query();

        let images = util::select(&doc, query.images.image);
        let mut attrs = query.images.image_attrs.clone().unwrap_or(vec![]);
        let mut default_attrs = vec!["src", "data-src"];
        attrs.append(&mut default_attrs);

        let images = images
            .elements
            .into_iter()
            .map(|img| self.abs_url(&loc, &img, &attrs.to_vec()))
            .collect::<Result<Vec<Url>>>()?;

        Ok(images)
    }

    async fn images_from_url(&self, url: &Url) -> Result<Vec<Url>> {
        let (_, doc_loc) = self.get_document_from_url(url).await?;

        self.get_images(doc_loc)
    }

    fn normalize_title(&self, title: &str) -> String {
        NORMALIZE_TITLE_MANGA.replace_all(title, "").trim().to_owned()
    }

    fn parse_keywords(&self, _hostname: &str, keywords: &str) -> String {
        if let Some(GenericQuerySearch { encode, .. }) = self.get_query().search {
            if encode {
                return urlencoding::encode(keywords).into_owned();
            }
        }
        keywords.to_owned()
    }
    fn parse_search_url(&self, hostname: &str, keywords: &str, path: &str) -> Result<Url> {
        let path = path.trim_start_matches("/");
        let path = path.replace("[query]", &self.parse_keywords(hostname, keywords));
        let url = format!("https://{}/{}", hostname, path);
        let url = Url::parse(&url).map_err(|_| ParseError::InvalidSearchUrl(url))?;

        Ok(url)
    }
    async fn do_search(
        &self,
        keywords: String,
        hostnames: Vec<String>,
    ) -> Result<Vec<SearchManga>> {
        let query = self.get_query();
        let query = (&query.search.as_ref()).ok_or(ParseError::SearchNotImplemented)?;
        let mut searchable_hostnames = self.hostnames();

        if let Some(hostnames) = &query.hostnames {
            searchable_hostnames = hostnames.clone();
        }

        if searchable_hostnames.is_empty() {
            return Err(ParseError::SearchMissingHostnames);
        }

        let mut results = vec![];

        let path = query.path;
        for hostname in hostnames.iter() {
            // If hostname is searchable
            if searchable_hostnames.contains(&hostname.as_str()) {
                let url = self.parse_search_url(&hostname, &keywords, &path)?;
                debug!("searching: {}", url);
                // Add results to results array
                let result = self.get_document_from_url(&url).await;
                let doc = match result {
                    Err(e) => {
                        error!("{} {:?}", url, e);
                        continue;
                    }
                    Ok((_, (doc, _))) => doc,
                };

                let elements = util::select(&doc, query.base);

                let href_attrs = util::merge_attr_with_default(&query.href_attr, vec!["href"]);
                let cover_attrs =
                    util::merge_vec_with_default(&query.cover_attrs, vec!["src", "src-data"]);

                for element in elements.elements.iter() {
                    // Href
                    let href =
                        util::select_element_fallback(element, query.href, Some(element.clone()));
                    let href = href.ok_or(ParseError::MissingSearchHref)?;
                    let search_url = self.abs_url(&url, &href, &href_attrs)?;

                    // Title
                    let title = util::select_fallback(
                        element,
                        query.title,
                        query.title_attr,
                        Some(href.clone()),
                    )
                    .ok_or(ParseError::MissingSearchTitle)?;

                    // Posted
                    let posted = util::select_fallback(
                        element,
                        query.posted,
                        query.posted_attr,
                        Some(element.clone()),
                    )
                    .map_or(None, |date| util::try_parse_date(&date));

                    // Cover
                    let cover =
                        util::select_element_fallback(element, query.cover, Some(element.clone()));
                    let cover = if let Some(cover) = cover {
                        self.abs_url(&url, &cover, &cover_attrs).ok()
                    } else {
                        None
                    };

                    results.push(SearchManga {
                        url: search_url,
                        title,
                        posted,
                        cover,
                    });
                }
            }
        }
        Ok(results)
    }
    fn abs_url(&self, location: &Url, element: &Element, attrs: &Vec<&'static str>) -> Result<Url> {
        for attr in attrs.iter() {
            let url = &element.attr(attr);
            if let Some(url) = url {
                let result = Url::parse(&url.trim().to_string());
                if let Err(_) = result {
                    let mut base = location.clone();
                    base.set_path("/");
                    let url = base
                        .join(&url.to_string())
                        .map_err(|_| ParseError::FailedToMakeAbsolute(url.to_string()));
                    return url;
                } else {
                    return result.map_err(|_| ParseError::FailedToMakeAbsolute(url.to_string()));
                }
            }
        }
        return Err(ParseError::NoUrlFound(element.tag(), attrs.clone()));
    }
    fn accepts(&self, url: &Url) -> Result<()> {
        let hostname = util::get_hostname(&url)?;
        if self.get_query().hostnames.contains(&hostname.as_str()) {
            return Ok(());
        } else {
            return Err(ParseError::NotAccepted(url.to_string()));
        }
    }
    fn title(&self, (doc, _): &DocLoc) -> Result<String> {
        let query = self.get_query();
        let elements: Elements = util::select(doc, query.manga.title).into();

        if let Some(attr) = query.manga.title_attr {
            if let Some(title) = elements.attr(attr) {
                Ok(title)
            } else {
                elements.text().ok_or(ParseError::MissingMangaTitle)
            }
        } else {
            elements.text().ok_or(ParseError::MissingMangaTitle)
        }
    }
    fn description(&self, (doc, _): &DocLoc) -> Result<String> {
        let query = self.get_query();

        if let Some(description_query) = query.manga.description {
            let elements: Elements = util::select(doc, description_query).into();

            if let Some(attr) = query.manga.description_attr {
                if let Some(description) = elements.attr(attr) {
                    return Ok(description);
                }
            }
            Ok(elements.text().unwrap_or("No description".to_owned()))
        } else {
            return Err(ParseError::MissingQuery("description"));
        }
    }
    fn cover(&self, (doc, _): &DocLoc) -> Option<Url> {
        let query = self.get_query();

        if let Some(cover_query) = query.manga.cover {
            let elements: Elements = util::select(doc, cover_query).into();

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
            let elements: Elements = util::select(doc, ongoing_query).into();

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

    fn parser_can_search(&self) -> Option<Vec<String>> {
        let query = self.get_query();
        if let Some(search) = &query.search {
            let hostnames: Vec<String> = if let Some(hostnames) = &search.hostnames {
                hostnames.iter().map(|s| s.to_string()).collect()
            } else {
                query.hostnames.iter().map(|s| s.to_string()).collect()
            };
            if hostnames.is_empty() {
                None
            } else {
                Some(hostnames)
            }
        } else {
            None
        }
    }
    fn parser_hostnames(&self) -> Vec<&'static str> {
        self.get_query().hostnames.clone()
    }
    fn parser_rate_limit(&self) -> u32 {
        100
    }
}

#[derive(parser_macro_derive::ParserDerive)]
pub struct GenericQueryParser {
    query: GenericQuery,
}

impl GenericQueryParser {
    pub fn new(query: GenericQuery) -> Self
    where
        Self: Sized + Send + Sync,
    {
        Self { query }
    }
}

impl IGenericQueryParser for GenericQueryParser {
    fn get_query(&self) -> &GenericQuery {
        &self.query
    }
}
