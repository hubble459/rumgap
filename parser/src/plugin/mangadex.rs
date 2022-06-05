use anyhow::anyhow;
use async_trait::async_trait;
use mangadex_api::{
    types::{Language, RelationshipType},
    v5::MangaDexClient,
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

        let id = segments
            .next()
            .filter(|s| s == &"title" || s == &"manga")
            .ok_or(anyhow!("Can't parse this url"))?;

        let uuid = &uuid::Uuid::parse_str(id)?;

        let manga = self
            .client
            .manga()
            .get()
            .manga_id(uuid)
            .include(&mangadex_api::types::ReferenceExpansionResource::Author)
            .include(&mangadex_api::types::ReferenceExpansionResource::CoverArt)
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
            .aggregate()
            .translated_language(vec![Language::English])
            .manga_id(uuid)
            .build()?
            .send()
            .await?;

        let chapters: Vec<Chapter> = chapters
            .volumes
            .values()
            .flat_map(|volume| {
                volume.chapters.values().map(|chapter| Chapter {
                    number: chapter.chapter.parse().unwrap(),
                    posted: None,
                    title: chapter.id.to_string(),
                    url: Url::parse(&format!("https://{}/{}", mangadex_api::API_URL, chapter.id)).unwrap(),
                })
            })
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
            authors: vec![],
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
            ongoing: true,
        })
    }
    async fn chapters(&self, url: reqwest::Url) -> anyhow::Result<Vec<Chapter>> {
        todo!()
    }
    async fn images(&self, url: reqwest::Url) -> anyhow::Result<Vec<reqwest::Url>> {
        todo!()
    }
    async fn search(&self, keyword: reqwest::Url) -> anyhow::Result<Vec<Manga>> {
        todo!()
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
