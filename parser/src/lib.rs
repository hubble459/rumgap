pub mod model;
pub mod parser;
pub mod plugin;
mod util;
pub use reqwest::Url;

#[macro_use]
extern crate log;

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
            $name:ident: $url:expr$(, $strictness:literal)?;
        )*) => {
            $(
                /// [`url`]: $url
                #[tokio::test]
                async fn $name() {
                    let _ = env_logger::builder().write_style(env_logger::WriteStyle::Always).filter(Some("parser"), log::LevelFilter::Debug).is_test(true).try_init();

                    let url = Url::parse($url).unwrap();
                    let manga = PARSER.manga(url).await.unwrap();
                    #[allow(unused_mut, unused_assignments)]
                    let mut strictness = 0b111;
                    $(
                        strictness = $strictness;
                    )*
                    assert_manga(manga, strictness).await;
                }
            )*
        }
    }

    manga_tests! {
        // Madara
        isekaiscanmanga: "https://isekaiscanmanga.com/manga/cancel-this-wish";
        isekaiscan: "https://isekaiscan.com/manga/on-my-way-to-kill-god", 0b110;
        aquamanga: "https://aquamanga.com/read/my-insanely-competent-underlings", 0b100;
        mangaonlineteam: "https://mangaonlineteam.com/manga/miss-divine-doctor-conquer-the-demon-king/";
        mangarockteam: "https://mangarockteam.com/manga/above-ten-thousand-people";
        manhuaus: "https://manhuaus.com/manga/magic-emperor-0/";
        mangaweebs: "https://mangaweebs.in/manga/2dmgoc9v5rbcjrdng8ra/";
        manhuaplus: "https://manhuaplus.com/manga/ultimate-loading-system/", 0b110;
        // TODO: Fix AJAX Chapters
        mangasushi: "https://mangasushi.org/manga/shin-no-nakama-janai-to-yuusha-no-party-wo-oidasareta-node-henkyou-de-slow-life-suru-koto-ni-shimashita/";
        _1stkissmanga: "https://1stkissmanga.io/manga/outside-the-law/";
        mangafoxfull: "https://mangafoxfull.com/manga/magic-emperor/";

      // await testScraper("https://s2manga.com/manga/under-the-oak-tree/");
      // await testScraper("https://manhwatop.com/manga/magic-emperor/");
      // await testScraper("https://manga68.com/manga/magic-emperor/");
      // await testScraper("https://manga347.com/manga/magic-emperor/");
      // await testScraper("https://mixedmanga.com/manga/the-eunuchs-consort-rules-the-world/");
      // await testScraper("https://mangahz.com/read/the-eunuchs-consort-rules-the-world/");
      // await testScraper("https://manhuadex.com/manhua/the-eunuchs-consort-rules-the-world/");
      // await testScraper("https://mangachill.com/manga/the-eunuchs-consort-rules-the-world/");
      // await testScraper("https://mangarockteam.com/manga/the-eunuchs-consort-rules-the-world/");
      // await testScraper("https://mangazukiteam.com/manga/the-eunuchs-consort-rules-the-world/");
      // await testScraper("https://azmanhwa.net/manga/the-eunuchs-consort-rules-the-world/");
      // await testScraper("https://topmanhua.com/manga/lightning-degree/");
      // await testScraper("https://yaoi.mobi/manga/stack-overflow-raw-yaoi0003/");
      // await testScraper("https://mangafunny.com/manga/past-lives-of-the-thunder-god/");
      // await testScraper("https://mangatx.com/manga/lightning-degree/");


        // Other
        read_m: "https://readm.org/manga/19309";
        reaperscans: "https://reaperscans.com/comics/5601-the-tutorial-is-too-hard", 0;

        // JSON API
        mangadex: "https://mangadex.org/title/56a35035-3c2c-4220-a764-a5d92d470e51/danshikou-ni-dokusareta-otokonoko";
    }

    async fn assert_manga(manga: Manga, strictness: u8) {
        assert!(!manga.title.is_empty(), "Title is empty");
        assert!(!manga.description.is_empty(), "Description is empty");
        assert_ne!(manga.description, "No description", "Description is empty");
        assert!(manga.ongoing, "Manga is not ongoing");
        assert!(manga.url.has_host(), "Url is missing host");
        if strictness & 0b100 == 0b100 {
            assert!(!manga.genres.is_empty(), "Missing genres");
        }
        if strictness & 0b010 == 0b010 {
            assert!(!manga.authors.is_empty(), "Missing authors");
        }
        if strictness & 0b001 == 0b001 {
            assert!(!manga.alt_titles.is_empty(), "Missing alternative titles");
        }
        assert!(!manga.chapters.is_empty(), "Missing chapters");

        let first_chapter = manga.chapters.first().unwrap();
        let images = PARSER.images(&first_chapter.url).await;
        assert!(images.is_ok(), "error: {:#?}", images.unwrap_err());
        let images = images.unwrap();
        assert!(!images.is_empty(), "No images found in chapter");
    }
}
