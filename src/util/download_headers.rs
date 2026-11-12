//! Extra headers for downloading images from scraped sites, on top of manga_parser's
//! `HTTP_CLIENT` defaults (which include a browser User-Agent).

use axum::http::header::USER_AGENT;
use axum::http::{HeaderMap, HeaderValue};
use manga_parser::Url;

/// MangaDex requires an honest User-Agent, and answers a browser one that doesn't run its
/// JavaScript with a bot check instead of the image. Use with `.headers()`, which replaces
/// the default User-Agent (`.header()` would add a second one).
pub fn for_url(url: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let host = Url::parse(url).ok().and_then(|url| url.host_str().map(str::to_string));
    if host.is_some_and(|host| host == "mangadex.org" || host.ends_with(".mangadex.org")) {
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(concat!(
                "rumgap/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/hubble459/rumgap)"
            )),
        );
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_mangadex_gets_its_own_user_agent() {
        assert!(for_url("https://uploads.mangadex.org/covers/a/b.jpg").contains_key(USER_AGENT));
        assert!(for_url("https://mangadex.org/title/a").contains_key(USER_AGENT));
        assert!(for_url("https://notmangadex.org/a.jpg").is_empty());
        assert!(for_url("https://weebcentral.com/a.jpg").is_empty());
        assert!(for_url("not a url").is_empty());
    }
}
