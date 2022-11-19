extern crate parser;

use once_cell::sync::Lazy;
use reqwest::Url;

use parser::{
    model::Manga,
    parse_error::{ParseError, Result},
    parser::{MangaParser, Parser},
};

#[macro_use]
extern crate log;

static PARSER: Lazy<MangaParser> = Lazy::new(|| MangaParser::new());

macro_rules! manga_tests {
        ($(
            $(#[$meta:meta])*
            $name:ident: $url:expr$(, $strictness:literal)?;
        )*) => {
            $(
                /// [`url`]: $url
                #[tokio::test]
                $(#[$meta])*
                async fn $name() {
                    let _ = env_logger::builder()
                        .write_style(env_logger::WriteStyle::Always)
                        .filter(Some("parser"), log::LevelFilter::Debug)
                        .is_test(true)
                        .try_init();

                    let url = Url::parse($url).unwrap();
                    let cloned_url = url.clone();
                    let hostname = cloned_url.domain().unwrap();
                    let manga = PARSER.manga(url).await;
                    if let Err(e) = manga {
                        match e {
                            ParseError::CloudflareIUAM => {
                                error!("[{}]: {}", hostname, ParseError::CloudflareIUAM);
                            },
                            ParseError::NetworkError(status) => {
                                error!("[{}]: {}", hostname, status);
                            },
                            ParseError::NetworkErrorUnknown(e) => {
                                error!("[{}]: {}", hostname, e);
                            },
                            _ => {
                                error!("[{}]: {}", hostname, e.to_string());
                                assert!(false);
                            },
                        }
                        return;
                    }
                    let manga = manga.unwrap();
                    let strictness = {0b111$(;$strictness)*};
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
    manhuaus: "https://manhuaus.com/manga/magic-emperor-0/";
    mangaweebs: "https://mangaweebs.in/manga/2dmgoc9v5rbcjrdng8ra/";
    manhuaplus: "https://manhuaplus.com/manga/ultimate-loading-system/", 0b110;
    mangasushi: "https://mangasushi.org/manga/shin-no-nakama-janai-to-yuusha-no-party-wo-oidasareta-node-henkyou-de-slow-life-suru-koto-ni-shimashita/";
    mangafoxfull: "https://mangafoxfull.com/manga/magic-emperor/";
    // TODO: Fix CloudFlare IUAM
    #[ignore = "cloudflare"]
    _1stkissmanga: "https://1stkissmanga.io/manga/outside-the-law/";
    #[ignore = "cloudflare"]
    s2manga: "https://s2manga.com/manga/under-the-oak-tree/";
    manhwatop: "https://manhwatop.com/manga/magic-emperor/";
    manga68: "https://manga68.com/manga/magic-emperor/";
    mixedmanga: "https://mixedmanga.com/manga/the-eunuchs-consort-rules-the-world/";
    mangahz: "https://mangahz.com/read/the-eunuchs-consort-rules-the-world/";
    manhuadex: "https://manhuadex.com/manhua/the-eunuchs-consort-rules-the-world/";
    mangachill: "https://mangachill.io/manga/the-eunuchs-consort-rules-thechbacc/";
    mangarockteam: "https://mangarockteam.com/manga/above-ten-thousand-people";
    mangazukiteam: "https://mangazukiteam.com/manga/the-eunuchs-consort-rules-the-world/";
    azmanhwa: "https://azmanhwa.net/manga/hazure-skill-ga-cha-de-tsuiho-sareta-ore-ha-waga-mama-osananajimi-wo-zetsuen-shi-kakusei-suru-banno-chi-toss-kill-wo-get-shite-mezase-rakuraku-saikyo-slow-life";
    topmanhua: "https://topmanhua.com/manga/lightning-degree/", 0b110;
    yaoi: "https://yaoi.mobi/manga/stack-overflow-raw-yaoi0003/", 0b100;
    mangatx: "https://mangatx.com/manga/lightning-degree/";


    // Other
    read_m: "https://readm.org/manga/19309";
    reaperscans: "https://reaperscans.com/comics/5601-the-tutorial-is-too-hard", 0;
    manga347: "https://manga347.com/manga/the-ultimate-of-all-ages/15", 0;

    // JSON API
    mangadex: "https://mangadex.org/title/19a107f1-7e6e-487e-8ab0-19c2618d9cd2/peter-grill-and-the-philosopher-s-time";
}

#[tokio::test]
#[ignore = "is for quick testing"]
async fn tmp_test() -> Result<()> {
    let _ = env_logger::builder()
        .write_style(env_logger::WriteStyle::Always)
        .filter(Some("parser"), log::LevelFilter::Debug)
        .is_test(true)
        .try_init();
    let url = Url::parse("https://azmanhwa.net/manga/hazure-skill-ga-cha-de-tsuiho-sareta-ore-ha-waga-mama-osananajimi-wo-zetsuen-shi-kakusei-suru-banno-chi-toss-kill-wo-get-shite-mezase-rakuraku-saikyo-slow-life/chapter-16-1").unwrap();
    let images = PARSER.images(&url).await?;
    assert!(!images.is_empty(), "No images found in chapter");

    Ok(())
}

/// strictness flags
/// - [0b100] = genres
/// - [0b010] = authors
/// - [0b001] = alt_titles
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
    let images = PARSER.images(&first_chapter.url).await.unwrap();
    assert!(!images.is_empty(), "No images found in chapter");
}
