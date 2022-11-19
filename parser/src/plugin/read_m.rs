use super::generic_query_parser::GenericQueryParser;
use crate::model::{GenericQuery, GenericQueryImages, GenericQueryManga, GenericQueryMangaChapter};

pub struct ReadM;

impl ReadM {
    pub fn new() -> GenericQueryParser {
        GenericQueryParser::new(GenericQuery {
            manga: GenericQueryManga {
                title: "h1.page-title",
                description: Some("p span"),
                cover: Some("img.series-profile-thumb"),
                is_ongoing: Some("span.series-status.aqua"),
                alt_titles: Some("div.sub-title.pt-sm"),
                authors: Some("#first_episode a small"),
                genres: Some("div.series-summary-wrapper div.ui.list div.item a"),
                chapter: GenericQueryMangaChapter {
                    base: "div.season_start tr",
                    href: Some("td > h6 > a"),
                    posted: Some("td.episode-date"),
                    ..Default::default()
                },
                ..Default::default()
            },
            images: GenericQueryImages {
                image: "center img",
                ..Default::default()
            },
            search: None,
            hostnames: vec!["readm.org"],
            ..Default::default()
        })
    }
}
