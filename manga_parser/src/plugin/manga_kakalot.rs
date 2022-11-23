use regex::Regex;
use reqwest::Url;

use super::generic_query_parser::{DocLoc, GenericQueryParser, IGenericQueryParser};
use crate::{
    model::{
        GenericQuery, GenericQueryImages, GenericQueryManga, GenericQueryMangaChapter,
        GenericQuerySearch,
    },
    parse_error::{ParseError, Result},
    util,
};

lazy_static! {
    static ref REGEX_SPECIAL_CHARACTERS: Regex = Regex::new(r"\W").unwrap();
}

#[derive(parser_macro_derive::ParserDerive)]
pub struct MangaKakalot {
    query: GenericQuery,
    parser: GenericQueryParser,
}

impl MangaKakalot {
    pub fn new() -> Self {
        let query = GenericQuery {
            manga: GenericQueryManga {
                title: "h1",
                description: Some("#noidungm, #panel-story-info-description, #example2, div:has(> h2:icontains(sum)), div:has(> h3:icontains(desc))"),
                cover: Some("#primaryimage, div.manga-info-pic > img"),
                cover_attrs: Some(vec!["data-src", "src"]),
                is_ongoing: Some("li:icontains(status), td:icontains(status) + td"),
                genres: Some("li:icontains(genre) > a, td:icontains(genre) + td a"),
                alt_titles: Some("h2:icontains(alt), h2.story-alternative, td:icontains(alt) + td"),
                authors: Some("li:icontains(author) > a, td:icontains(author) + td a"),
                chapter: GenericQueryMangaChapter {
                    base: "div.chapter-list div.row, div.chapter h4, ul.row-content-chapter li",
                    href: Some("span a, a"),
                    posted: Some("span[title]"),
                    posted_attr: Some("title"),
                    ..Default::default()
                },
                ..Default::default()
            },
            images: GenericQueryImages {
                image: "div.container-chapter-reader img, div.vung-doc img",
                ..Default::default()
            },
            search: Some(
                GenericQuerySearch {
                    path: "/search/story/[query]",
                    base: "div.story_item, div.list-story-item, div.mainpage-manga",
                    href: Some("h3 > a, div.media-body a"),
                    title: Some("div.media-body a h4"),
                    posted: Some("span:icontains(updated), div.hotup-list i"),
                    cover: Some("a img"),
                    encode: false,
                    ..Default::default()
                }
            ),
            hostnames: vec![
                "mangabat.com",
                "mangabat.best",
                "mangakakalot.com",
                "mangakakalot.tv",
                "manganelo.com",
                "manganato.com",
                "readmanganato.com",
            ],
            ..Default::default()
        };
        Self {
            parser: GenericQueryParser::new(query.clone()),
            query,
        }
    }
}

impl IGenericQueryParser for MangaKakalot {
    fn get_query(&self) -> &GenericQuery {
        &self.query
    }

    fn parse_keywords(&self, _hostname: &str, keywords: &str) -> String {
        REGEX_SPECIAL_CHARACTERS
            .replace_all(&keywords.replace(" ", "_"), "")
            .into_owned()
    }

    fn get_images(&self, (doc, loc): DocLoc) -> Result<Vec<Url>> {
        let hostname = util::get_hostname(&loc)?;

        if hostname == "mangabat.best" {
            let element =
                util::select_first(&doc, "#arraydata").ok_or(ParseError::MissingImages)?;
            let images = element.text().ok_or(ParseError::MissingImages)?;

            return images
                .split(",")
                .map(|url| Url::parse(url).map_err(|_| ParseError::MissingImages))
                .collect::<core::result::Result<Vec<Url>, _>>();
        }

        self.parser.get_images((doc, loc))
    }

    fn parse_search_url(&self, hostname: &str, keywords: &str, path: &str) -> Result<Url> {
        let mut path = path.to_owned();
        match hostname {
            "mangabat.best" => {
                path = String::from("/search?q=[query]");
            }
            "mangabat.com" => {
                path = String::from("/search/manga/[query]");
            }
            _ => {}
        }

        let url = self.parser.parse_search_url(hostname, &self.parse_keywords(hostname, &keywords), &path);

        url.map(|mut url| {
            match hostname {
                "mangabat.best" => {
                    url.set_scheme("http").unwrap();
                }
                "mangabat.com" => {
                    url.set_host(Some("h.mangabat.com")).unwrap();
                }
                _ => {}
            }
            url
        })
    }
}
