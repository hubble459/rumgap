use super::generic_query_parser::GenericQueryParser;
use crate::model::{GenericQuery, GenericQueryImages, GenericQueryManga, GenericQueryMangaChapter};

pub struct ReaperScans;

impl ReaperScans {
    pub fn new() -> GenericQueryParser {
        GenericQueryParser::new(GenericQuery {
            manga: GenericQueryManga {
                title: "h1.font-semibold",
                description: Some("h1:icontains(summary) + p"),
                cover: Some("div.transition > img"),
                is_ongoing: Some("dt:icontains(status) + dd"),
                chapter: GenericQueryMangaChapter {
                    base: "ul[role=list] > li > a",
                    title: Some("div p"),
                    posted: Some("p:icontains(released)"),
                    ..Default::default()
                },
                ..Default::default()
            },
            images: GenericQueryImages {
                image: "main p > img",
                ..Default::default()
            },
            search: None,
            hostnames: vec!["reaperscans.com"],
            ..Default::default()
        })
    }
}
