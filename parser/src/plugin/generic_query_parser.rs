use crate::{
    model::{Chapter, GenericQuery, Manga, SearchManga},
    parser::Parser,
};
use anyhow::anyhow;
use reqwest::Url;
use visdom::{
    types::{BoxDynElement, Elements},
    Vis,
};

pub struct GenericQueryParser {
    query: GenericQuery,
}

pub type DocLoc<'html> = (Elements<'html>, Url);

#[async_trait::async_trait]
pub trait IGenericQueryParser {
    // Getters
    fn get_query() -> GenericQuery;
    fn get_document() -> {
        return Vis::load(html).map_err(|e| anyhow!(e));
    }
    // Async Functions
    async fn get_document_from_url(&self, url: Url) -> anyhow::Result<(String, DocLoc)> {
        let response = reqwest::get(url).await?;
        let url = response.url().clone();
        let html = response.text().await?;
        let document = get_document(&html)?;
        Ok((html.to_owned(), (document, url)))
    }
    async fn chapters(&self, html: &str, url: &Url, title: &String) -> Vec<Chapter>;
    fn abs_url(
        &self,
        location: &Url,
        element: &BoxDynElement,
        attrs: Vec<&'static str>,
    ) -> anyhow::Result<Url>;
    fn accepts(&self, url: &Url) -> anyhow::Result<()>;
    fn title(&self, doc_loc: &DocLoc) -> String;
    fn description(&self, doc_loc: &DocLoc) -> String;
    fn cover(&self, doc_loc: &DocLoc) -> Option<Url>;
    fn ongoing(&self, doc_loc: &DocLoc) -> bool;
    fn genres(&self, doc_loc: &DocLoc) -> Vec<String>;
    fn alt_titles(&self, doc_loc: &DocLoc) -> Vec<String>;
    fn authors(&self, doc_loc: &DocLoc) -> Vec<String>;
}

#[async_trait::async_trait]
impl IGenericQueryParser for GenericQueryParser {
    fn new(generic_query: GenericQuery) -> Self {
        GenericQueryParser {
            query: generic_query,
        }
    }

    fn get_document(html: &str) -> anyhow::Result<Elements> {
        return Vis::load(html).map_err(|e| anyhow!(e));
    }

    fn abs_url(
        &self,
        location: &Url,
        element: &BoxDynElement,
        attrs: Vec<&'static str>,
    ) -> anyhow::Result<Url> {
        for attr in attrs.iter() {
            let url = &element.get_attribute(*attr);
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
                    return anyhow::Ok(url);
                }
            }
        }
        anyhow::bail!("No url found in element (with attrs: {:?})", attrs);
    }

    fn accepts(&self, url: &Url) -> anyhow::Result<()> {
        if let Some(hostname) = url.host_str() {
            if self.query.hostnames.contains(&hostname) {
                return Ok(());
            } else {
                return Err(anyhow::format_err!(
                    "This parser does not support this url \"{:?}\"",
                    url.to_string()
                ));
            }
        } else {
            return Err(anyhow::format_err!("Url should have a hostname"));
        }
    }
    async fn chapters(&self, html: &str, url: &Url, title: &String) -> Vec<Chapter> {
        todo!()
    }
    fn title(&self, doc_loc: &DocLoc) -> String {
        todo!()
    }
    fn description(&self, doc_loc: &DocLoc) -> String {
        todo!()
    }
    fn cover(&self, doc_loc: &DocLoc) -> Option<Url> {
        todo!()
    }
    fn ongoing(&self, doc_loc: &DocLoc) -> bool {
        todo!()
    }
    fn genres(&self, doc_loc: &DocLoc) -> Vec<String> {
        todo!()
    }
    fn alt_titles(&self, doc_loc: &DocLoc) -> Vec<String> {
        todo!()
    }
    fn authors(&self, doc_loc: &DocLoc) -> Vec<String> {
        todo!()
    }
}

#[async_trait::async_trait]
impl Parser for GenericQueryParser {
    async fn manga(&self, url: Url) -> anyhow::Result<Manga> {
        self.accepts(&url)?;

        let (html, mut manga) = {
            let (html, doc_loc) = &self
                .get_document_from_url(url.clone())
                .await
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;

            Ok((
                html.to_owned(),
                Manga {
                    url: doc_loc.1.clone(),
                    title: self.title(doc_loc),
                    description: self.description(doc_loc),
                    cover: self.cover(doc_loc),
                    ongoing: self.ongoing(doc_loc),
                    genres: self.genres(doc_loc),
                    authors: self.authors(doc_loc),
                    alt_titles: self.alt_titles(doc_loc),
                    chapters: vec![],
                },
            ))
        }?;

        manga.chapters = self.chapters(&html, &manga.url, &manga.title).await;

        Ok(manga)
    }

    async fn images(&self, url: Url) -> anyhow::Result<Vec<Url>> {
        let (_, (doc, location)) = self
            .get_document_from_url(url)
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;

        let images = doc.filter(self.query.images.image);
        let mut attrs = self.query.images.image_attrs.clone().unwrap_or(vec![]);
        let mut default_attrs = vec!["src", "data-src"];
        attrs.append(&mut default_attrs);

        let images = images
            .into_iter()
            .map(|img| self.abs_url(&location, &img, attrs.to_vec()))
            .collect::<anyhow::Result<Vec<Url>>>()?;

        Ok(images)
    }
    async fn search(
        &self,
        keyword: String,
        hostnames: Vec<String>,
    ) -> anyhow::Result<Vec<SearchManga>> {
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
