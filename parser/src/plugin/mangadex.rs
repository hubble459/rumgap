use std::vec;

use anyhow::anyhow;
use async_trait::async_trait;
use itertools::Itertools;
use mangadex_api::{
    types::{Language, MangaStatus, RelationshipType},
    v5::{schema::RelatedAttributes, MangaDexClient},
};
use reqwest::Url;

use crate::{model::*, parser::Parser};

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
    async fn manga(&self, url: reqwest::Url) -> anyhow::Result<Manga> {
        let mut segments = url.path_segments().ok_or(anyhow!("Can't parse this url"))?;

        segments
            .next()
            .filter(|s| s == &"title" || s == &"manga")
            .ok_or(anyhow!("Can't parse this url"))?;

        let uuid = &uuid::Uuid::parse_str(segments.next().ok_or(anyhow!("No ID found in url"))?)?;

        let manga = self
            .client
            .manga()
            .get()
            .manga_id(uuid)
            .include(&mangadex_api::types::ReferenceExpansionResource::Author)
            .build()?
            .send()
            .await?;

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
                .build()?
                .send()
                .await?;

            Some(Url::parse(&format!(
                "{}/covers/{}/{}",
                mangadex_api::constants::CDN_URL,
                uuid,
                cover.data.attributes.file_name
            ))?)
        } else {
            None
        };

        let chapters = self
            .client
            .manga()
            .feed()
            .limit(100 as u32)
            .translated_language(vec![Language::English])
            .manga_id(uuid)
            .build()?
            .send()
            .await??;

        let chapters: Vec<Chapter> = chapters
            .data
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
                url: Url::parse(&format!(
                    "https://{}/chapter/{}",
                    mangadex_api::API_URL,
                    chapter.id
                ))
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
                .ok_or(anyhow!("No title"))?
                .to_owned(),
            description: manga
                .data
                .attributes
                .description
                .get(&mangadex_api::types::Language::English)
                .ok_or(anyhow!("No description"))?
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
    async fn images(&self, url: reqwest::Url) -> anyhow::Result<Vec<reqwest::Url>> {
        let mut segments = url.path_segments().ok_or(anyhow!("Can't parse this url"))?;

        segments
            .next()
            .filter(|s| s == &"title" || s == &"manga")
            .ok_or(anyhow!("Can't parse this url"))?;

        let uuid = &uuid::Uuid::parse_str(segments.next().ok_or(anyhow!("No ID found in url"))?)?;

        let at_home = self
            .client
            .at_home()
            .server()
            .chapter_id(uuid)
            .build()?
            .send()
            .await?;

        let images: Vec<Url> = at_home
            .chapter
            .data
            .iter()
            .map(|filename| {
                at_home
                    .base_url
                    .join(&format!(
                        "/{quality_mode}/{chapter_hash}/{page_filename}",
                        quality_mode = "data",
                        chapter_hash = at_home.chapter.hash,
                        page_filename = filename
                    ))
                    .unwrap()
            })
            .collect();

        Ok(images)
    }
    async fn search(
        &self,
        keyword: &'static str,
        _hostnames: Vec<&'static str>,
    ) -> anyhow::Result<Vec<SearchManga>> {
        let results = self
            .client
            .search()
            .manga()
            .add_available_translated_language(Language::English)
            .title(keyword)
            .build()?
            .send()
            .await?;

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
                updated: m.attributes.updated_at.as_ref().map(|date| *date.as_ref()),
                cover: None,
                url: Url::parse(&format!("{}/manga/{}", mangadex_api::API_URL, m.id)).unwrap(),
            })
            .collect();

        Ok(search_results)
    }
    fn hostnames(&self) -> Vec<&'static str> {
        vec!["api.mangadex.org", "mangadex.org"]
    }

    fn can_search(&self) -> bool {
        true
    }

    fn rate_limit(&self) -> u32 {
        0
    }
}
