use std::vec;

use async_trait::async_trait;
use itertools::Itertools;
use mangadex_api::{
    types::{Language, MangaStatus, ReferenceExpansionResource, RelationshipType},
    v5::{
        schema::{ChapterObject, RelatedAttributes},
        MangaDexClient,
    },
};
use reqwest::Url;
use tokio::time::{sleep, Duration};

use crate::{
    model::*,
    parse_error::{ParseError, Result},
    parser::Parser,
};

pub struct MangaDex {
    client: MangaDexClient,
}

impl MangaDex {
    pub fn new() -> Self {
        MangaDex {
            client: MangaDexClient::default(),
        }
    }
}

#[async_trait]
impl Parser for MangaDex {
    async fn manga(&self, url: Url) -> Result<Manga> {
        let mut segments = url
            .path_segments()
            .ok_or(ParseError::NotAccepted(url.to_string()))?;

        segments
            .next()
            .filter(|s| s == &"title" || s == &"manga")
            .ok_or(ParseError::NotAccepted(url.to_string()))?;

        let uuid = &uuid::Uuid::parse_str(segments.next().ok_or(ParseError::NotAccepted(
            format!("No ID found in url ({})", url.as_str()),
        ))?)
        .map_err(|e| ParseError::Other(e.into()))?;

        let manga = self
            .client
            .manga()
            .get()
            .manga_id(uuid)
            .include(&mangadex_api::types::ReferenceExpansionResource::Author)
            .build()
            .map_err(|e| ParseError::Other(e.into()))?
            .send()
            .await
            .map_err(|e| ParseError::Other(e.into()))?;

        let cover_id = manga
            .data
            .relationships
            .iter()
            .find(|related| related.type_ == RelationshipType::CoverArt);

        let cover = if let Some(relationship) = cover_id {
            let cover = self
                .client
                .cover()
                .get()
                .cover_id(&relationship.id)
                .build()
                .map_err(|e| ParseError::Other(e.into()))?
                .send()
                .await
                .map_err(|e| ParseError::Other(e.into()))?;

            Some(
                Url::parse(&format!(
                    "{}/covers/{}/{}",
                    mangadex_api::constants::CDN_URL,
                    uuid,
                    cover.data.attributes.file_name
                ))
                .map_err(|e| ParseError::Other(e.into()))?,
            )
        } else {
            None
        };

        let mut chapters: Vec<ChapterObject> = vec![];
        let mut offset: u32 = 0;
        let mut total: u32 = 0;

        while offset == 0 || offset < total {
            if offset != 0 && offset % 2000 == 0 {
                // When 3 requests are made, wait one second before making the next
                sleep(Duration::from_secs(1)).await;
            }

            let results = self
                .client
                .manga()
                .feed()
                .limit(500u32)
                .offset(offset)
                .translated_language(vec![Language::English])
                .manga_id(uuid)
                .build()
                .map_err(|e| ParseError::Other(e.into()))?
                .send()
                .await
                .map_err(|e| ParseError::Other(e.into()))?
                .map_err(|e| ParseError::Other(e.into()))?;
            chapters.append(&mut results.data.clone());
            total = results.total;
            offset += 500;
        }

        let chapters: Vec<Chapter> = chapters
            .iter()
            .map(|chapter| Chapter {
                number: chapter
                    .attributes
                    .chapter
                    .as_ref()
                    .unwrap_or(&"-1".to_owned())
                    .parse()
                    .unwrap(),
                posted: Some(*chapter.attributes.created_at.as_ref()),
                title: chapter.attributes.title.to_owned(),
                url: Url::parse(&format!("{}/chapter/{}", mangadex_api::API_URL, chapter.id))
                    .unwrap(),
            })
            .sorted_by(|c1, c2| c1.posted.unwrap().cmp(&c2.posted.unwrap()))
            .collect();

        Ok(Manga {
            url,
            cover,
            title: manga
                .data
                .attributes
                .title
                .get(&mangadex_api::types::Language::English)
                .ok_or(ParseError::MissingMangaTitle)?
                .to_owned(),
            description: manga
                .data
                .attributes
                .description
                .get(&mangadex_api::types::Language::English)
                .unwrap_or(&"No description".to_owned())
                .to_owned(),
            alt_titles: manga
                .data
                .attributes
                .alt_titles
                .iter()
                .flat_map(|a| a.values().map(|a| a.to_owned()).collect::<Vec<String>>())
                .collect(),
            authors: manga
                .data
                .relationships
                .iter()
                .filter(|a| a.type_ == RelationshipType::Author)
                .map(|a| {
                    if let Some(RelatedAttributes::Author(author)) = &a.attributes {
                        Some(author.name.to_owned())
                    } else {
                        None
                    }
                })
                .filter(|a| a.is_some())
                .map(|a| a.unwrap().to_owned())
                .collect(),
            genres: manga
                .data
                .attributes
                .tags
                .iter()
                .filter(|a| a.type_ == RelationshipType::Tag)
                .map(|a| a.attributes.name.values().next())
                .filter(|a| a.is_some())
                .map(|a| a.unwrap().to_owned())
                .collect(),
            chapters,
            ongoing: manga.data.attributes.status == MangaStatus::Ongoing,
        })
    }
    async fn images(&self, url: &Url) -> Result<Vec<Url>> {
        let mut segments = url
            .path_segments()
            .ok_or(ParseError::NotAccepted(url.to_string()))?;

        segments
            .next()
            .filter(|s| s == &"chapter")
            .ok_or(ParseError::NotAccepted(url.to_string()))?;

        let uuid = &uuid::Uuid::parse_str(segments.next().ok_or(ParseError::NotAccepted(
            format!("No ID found in url ({})", url.as_str()),
        ))?)
        .map_err(|e| ParseError::Other(e.into()))?;

        let at_home = self
            .client
            .at_home()
            .server()
            .chapter_id(uuid)
            .build()
            .map_err(|e| ParseError::Other(e.into()))?
            .send()
            .await
            .map_err(|e| ParseError::Other(e.into()))?;

        let images: Vec<Url> = at_home
            .chapter
            .data_saver
            .iter()
            .map(|filename| {
                at_home
                    .base_url
                    .join(&format!(
                        "/{quality_mode}/{chapter_hash}/{page_filename}",
                        quality_mode = "data-saver",
                        chapter_hash = at_home.chapter.hash,
                        page_filename = filename
                    ))
                    .unwrap()
            })
            .collect();

        Ok(images)
    }
    async fn search(&self, keyword: String, _hostnames: Vec<String>) -> Result<Vec<SearchManga>> {
        let results = self
            .client
            .search()
            .manga()
            .add_available_translated_language(Language::English)
            .title(keyword.as_str())
            .include(ReferenceExpansionResource::CoverArt)
            .build()
            .map_err(|e| ParseError::Other(e.into()))?
            .send()
            .await
            .map_err(|e| ParseError::Other(e.into()))?;

        let search_results = results
            .data
            .iter()
            .map(|m| SearchManga {
                title: m
                    .attributes
                    .title
                    .get(&mangadex_api::types::Language::English)
                    .unwrap_or(&"No title".to_owned())
                    .to_owned(),
                posted: m.attributes.updated_at.as_ref().map(|date| *date.as_ref()),
                cover: m
                    .relationships
                    .clone()
                    .into_iter()
                    .find(|rel| rel.type_ == RelationshipType::CoverArt)
                    .map(|cover_rel| {
                        if let Some(RelatedAttributes::CoverArt(cover)) = cover_rel.attributes {
                            Url::parse(&format!(
                                "{}/covers/{}/{}",
                                mangadex_api::constants::CDN_URL,
                                m.id,
                                cover.file_name
                            ))
                            .unwrap()
                        } else {
                            panic!();
                        }
                    }),
                url: Url::parse(&format!("{}/manga/{}", mangadex_api::API_URL, m.id)).unwrap(),
            })
            .collect();

        Ok(search_results)
    }
    fn hostnames(&self) -> Vec<&'static str> {
        vec!["api.mangadex.org", "mangadex.org"]
    }

    fn can_search(&self) -> Option<Vec<String>> {
        Some(vec!["mangadex.org".to_owned()])
    }

    fn rate_limit(&self) -> u32 {
        0
    }
}
