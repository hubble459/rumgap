use super::generic_query_parser::{GenericQueryParser, IGenericQueryParser};

use crate::model::{GenericQuery, GenericQueryImages, GenericQueryManga};

pub struct Madara;

impl Madara {
    pub fn new() -> GenericQueryParser {
        GenericQueryParser::new(GenericQuery {
            manga: GenericQueryManga {
                title: "h1",
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
