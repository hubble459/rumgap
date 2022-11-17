pub mod model;
pub mod parser;
pub mod plugin;
pub use reqwest::Url;
mod util;

#[macro_use]
extern crate macro_rules_attribute;

#[cfg(test)]
mod tests {
    use once_cell::sync::Lazy;
    use reqwest::Url;

    use crate::{
        model::Manga,
        parser::{MangaParser, Parser},
    };

    static PARSER: Lazy<MangaParser> = Lazy::new(|| MangaParser::new());

    macro_rules! manga_tests {
        ($(
            $(#[$meta:meta])*
            $name:ident: $url:expr,
        )*) => {
            $(
                /// [`url`]: $url
                #[tokio::test]
                async fn $name() {
                    let url = Url::parse($url).unwrap();
                    let manga = PARSER.manga(url).await.unwrap();
                    assert_manga(manga).await;
                }
            )*
        }
    }

    async fn assert_manga(manga: Manga) {
        assert!(!manga.title.is_empty(), "Title is empty");
        assert!(!manga.description.is_empty(), "Description is empty");
        assert!(manga.ongoing, "Manga is not ongoing");
        assert!(manga.url.has_host(), "Url is missing host");
        assert!(!manga.authors.is_empty(), "Missing authors");
        assert!(!manga.genres.is_empty(), "Missing genres");
        assert!(!manga.alt_titles.is_empty(), "Missing alternative titles");
        assert!(!manga.chapters.is_empty(), "Missing chapters");

        let first_chapter = manga.chapters.first().unwrap();
        let images = PARSER.images(first_chapter.url.clone()).await;
        assert!(images.is_ok(), "error: {:#?}", images.unwrap_err());
        let images = images.unwrap();
        assert!(!images.is_empty(), "No images found in chapter");
    }

    manga_tests! {
        // Madara
        isekaiscanmanga: "https://isekaiscanmanga.com/manga/cancel-this-wish",

        // JSON API
        mangadex: "https://mangadex.org/title/56a35035-3c2c-4220-a764-a5d92d470e51/danshikou-ni-dokusareta-otokonoko",
    }
}
