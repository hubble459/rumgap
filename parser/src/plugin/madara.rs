use super::generic_query_parser::{GenericQueryParser, IGenericQueryParser};

use crate::model::{GenericQuery, GenericQueryImages, GenericQueryManga, GenericQueryMangaChapter};

pub struct Madara;

impl Madara {
    pub fn new() -> GenericQueryParser {
        GenericQueryParser::new(GenericQuery {
            manga: GenericQueryManga {
                title: "h1",
                description: Some("div.description-summary h3"),
                is_ongoing: Some("div.summary-heading:has(h5:icontains(status)) + div"),
                cover: Some("div.summary_image img.lazyloaded"),
                cover_attrs: Some(vec!["data-src"]),
                authors: Some("div.author-content a"),
                genres: Some("div.genres-content a"),
                alt_titles: Some("div.summary-heading:has(h5:icontains(alt)) + div"),
                chapter: GenericQueryMangaChapter {
                    base: "li.wp-manga-chapter",
                    href: "a",
                    posted: Some("i"),
                    ..Default::default()
                },
                ..Default::default()
            },
            images: GenericQueryImages {
                image: "div img.wp-manga-chapter-img",
                ..Default::default()
            },
            search: None,
            hostnames: vec!["isekaiscanmanga.com"],
            ..Default::default()
        })
    }
}
