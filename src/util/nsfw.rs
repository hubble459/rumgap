//! Adult-only sites, from the manga_parser configs marked `nsfw: true`. A source from one
//! of them marks its manga 18+ (`manga.nsfw`).

use manga_parser::scraper::MangaScraper;

use crate::MANGA_PARSER;

pub fn is_nsfw_hostname(hostname: &str) -> bool {
    MANGA_PARSER
        .nsfw_hostnames()
        .iter()
        .any(|nsfw| nsfw.eq_ignore_ascii_case(hostname))
}
