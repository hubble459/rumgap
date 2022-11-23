extern crate manga_parser;

use chrono::{DateTime, Duration, Months, Utc};
use reqwest::Url;

use manga_parser::{
    model::Manga,
    parse_error::{ParseError, Result},
    parser::{MangaParser, Parser},
    util,
};

#[macro_use]
extern crate log;
#[macro_use]
extern crate lazy_static;

lazy_static! {
    static ref PARSER: MangaParser = MangaParser::new();
    static ref CAN_SEARCH: Vec<String> = PARSER.can_search().unwrap();
}

fn init() {
    let _ = env_logger::builder()
        .write_style(env_logger::WriteStyle::Always)
        .filter(Some("manga-parser"), log::LevelFilter::Debug)
        .is_test(true)
        .try_init();
}

macro_rules! manga_tests {
        (
            $scraper:ident,
            $(
                $(#[$meta:meta])*
                $name:ident: $url:expr$(, $strictness:literal)?;
            )*
        ) => {
            mod $scraper {
                use crate::{init, PARSER, Url, ParseError, assert_manga};
                use crate::manga_parser::parser::Parser;

                $(
                    #[doc = "[`url`]: $url"]
                    #[tokio::test]
                    $(#[$meta])*
                    async fn $name() {
                        init();
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
                        let strictness = {0b1111$(;$strictness)*};
                        assert_manga(manga, strictness).await;
                    }
                )*
            }
        }
    }

manga_tests! {
    madara,
    isekai_scan_manga: "https://isekaiscanmanga.com/manga/cancel-this-wish";
    isekaiscan: "https://isekaiscan.com/manga/on-my-way-to-kill-god", 0b1110;
    aqua_manga: "https://aquamanga.com/read/my-insanely-competent-underlings", 0b1100;
    manga_online_team: "https://mangaonlineteam.com/manga/miss-divine-doctor-conquer-the-demon-king/";
    manhua_us: "https://manhuaus.com/manga/ghost-emperor/";
    manga_weebs: "https://mangaweebs.in/manga/2dmgoc9v5rbcjrdng8ra/";
    manhua_plus: "https://manhuaplus.com/manga/ultimate-loading-system/", 0b1110;
    manga_sushi: "https://mangasushi.org/manga/shin-no-nakama-janai-to-yuusha-no-party-wo-oidasareta-node-henkyou-de-slow-life-suru-koto-ni-shimashita/";
    manga_fox_full: "https://mangafoxfull.com/manga/magic-emperor/";
    _1stkiss_manga_club: "https://1stkissmanga.club/manga/outside-the-law/";
    #[ignore = "CloudFlare"]
    _1stkiss_manga_io: "https://1stkissmanga.io/manga/outside-the-law/";
    #[ignore = "CloudFlare"]
    s2_manga: "https://s2manga.com/manga/under-the-oak-tree/";
    manhwa_top: "https://manhwatop.com/manga/magic-emperor/";
    manga68: "https://manga68.com/manga/magic-emperor/";
    mixed_manga: "https://mixedmanga.com/manga/dungeon-start-by-enslaving-blue-star-players/", 0b1100;
    manga_hz: "https://mangahz.com/read/the-eunuchs-consort-rules-the-world/";
    manhua_dex: "https://manhuadex.com/manhua/the-eunuchs-consort-rules-the-world/";
    manga_chill: "https://mangachill.io/manga/the-eunuchs-consort-rules-thechbacc/";
    manga_rock_team: "https://mangarockteam.com/manga/above-ten-thousand-people";
    manga_zuki_team: "https://mangazukiteam.com/manga/shinjiteita-nakama-tachi-ni-dungeon/";
    az_manhwa: "https://azmanhwa.net/manga/hazure-skill-ga-cha-de-tsuiho-sareta-ore-ha-waga-mama-osananajimi-wo-zetsuen-shi-kakusei-suru-banno-chi-toss-kill-wo-get-shite-mezase-rakuraku-saikyo-slow-life";
    top_manhua: "https://topmanhua.com/manga/lightning-degree/", 0b110;
    yaoi: "https://yaoi.mobi/manga/stack-overflow-raw-yaoi0003/", 0b100;
    manga_tx: "https://mangatx.com/manga/lightning-degree/";
}

manga_tests! {
    misc,
    read_m: "https://readm.org/manga/19309";
    reaper_scans: "https://reaperscans.com/comics/5601-the-tutorial-is-too-hard", 0;
    manga347: "https://manga347.com/manga/the-ultimate-of-all-ages/15", 0;
    mangadex: "https://mangadex.org/title/19a107f1-7e6e-487e-8ab0-19c2618d9cd2/peter-grill-and-the-philosopher-s-time";
}

manga_tests! {
    manga_kakalot,

    manga_kakalot: "https://mangakakalot.com/manga/youkai_gakkou_no_sensei_hajimemashita";
    manga_bat_com: "https://h.mangabat.com/read-tj397750";
    manga_bat_best: "http://mangabat.best/manga/worthless-regression", 0b0101;
}

#[tokio::test]
#[ignore = "is for quick testing"]
async fn quick_test() -> Result<()> {
    init();

    Ok(())
}

/// strictness flags
/// - [0b01000] = chapter dates
/// - [0b00100] = genres
/// - [0b00010] = authors
/// - [0b00001] = alt_titles
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
    for chapter in manga.chapters.iter() {
        assert!(chapter.url.has_host(), "Chapter url is missing host");
        if strictness & 0b1000 == 0b1000 {
            assert!(chapter.posted.is_some(), "Chapter {} is missing a posted date", chapter.number);
        }
    }

    let first_chapter = manga.chapters.first().unwrap();
    let images = PARSER.images(&first_chapter.url).await.unwrap();
    assert!(!images.is_empty(), "No images found in chapter");

    let hostname = util::get_hostname(&manga.url).unwrap();
    if CAN_SEARCH.contains(&hostname) {
        let search_results = PARSER.search(manga.title.clone(), vec![hostname]).await;
        let search_results = search_results.unwrap();

        assert!(!search_results.is_empty(), "No search results");
        let item = search_results
            .into_iter()
            .find(|item| item.title.to_ascii_lowercase() == manga.title.to_ascii_lowercase());
        assert!(item.is_some(), "Could not find manga in search results");
        let item = item.unwrap();
        assert!(item.url.has_host(), "Search url is missing host");
    }
}

#[test]
fn date_parse() {
    init();
    let now = Utc::now();

    let date = manga_parser::util::try_parse_date(&now.timestamp_millis().to_string());
    compare_without_time(&now, date);

    let date = manga_parser::util::try_parse_date("Today");
    compare_without_time(&now, date);

    let date = manga_parser::util::try_parse_date("Hottest");
    compare_without_time(&now, date);

    let date = manga_parser::util::try_parse_date("Yesterday");
    compare_without_time(&(now - Duration::days(1)), date);

    let date = manga_parser::util::try_parse_date("Last week");
    compare_without_time(&(now - Duration::weeks(1)), date);

    let date = manga_parser::util::try_parse_date(&now.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string());
    compare_without_time(&now, date);

    let date = manga_parser::util::try_parse_date(&now.to_rfc3339());
    compare_without_time(&now, date);

    let date = manga_parser::util::try_parse_date(&now.format("%Y-%m-%dT%H:%M:%SZ").to_string());
    compare_without_time(&now, date);

    let date = manga_parser::util::try_parse_date(&now.format("%Y-%m-%dT%H:%M:%S").to_string());
    compare_without_time(&now, date);

    let date = manga_parser::util::try_parse_date(&now.format("%B %e, %Y").to_string());
    compare_without_time(&now, date);

    let date = manga_parser::util::try_parse_date(&now.format("%B %e, %Y").to_string());
    compare_without_time(&now, date);

    let date = manga_parser::util::try_parse_date(&now.format("%erd %B %Y").to_string());
    compare_without_time(&now, date);

    let date = manga_parser::util::try_parse_date(&now.format("%eth %B %Y").to_string());
    compare_without_time(&now, date);

    let date = manga_parser::util::try_parse_date(&now.format("%est %B %Y").to_string());
    compare_without_time(&now, date);

    let date = manga_parser::util::try_parse_date(&now.format("%end %B %Y").to_string());
    compare_without_time(&now, date);

    let date = manga_parser::util::try_parse_date(&now.format("%B %d %y - %H:%M").to_string());
    compare_without_time(&now, date);

    let date = manga_parser::util::try_parse_date(&now.format("%B %d %Y - %H:%M").to_string());
    compare_without_time(&now, date);

    let date = manga_parser::util::try_parse_date(&now.format("%B %d, %Y").to_string());
    compare_without_time(&now, date);

    let date = manga_parser::util::try_parse_date("about 1 Weeks ago!");
    compare_without_time(&(now - Duration::weeks(1)), date);

    let date = manga_parser::util::try_parse_date("15 Week");
    compare_without_time(&(now - Duration::weeks(15)), date);

    let date = manga_parser::util::try_parse_date("like 2 minutes ago");
    compare_without_time(&(now - Duration::minutes(2)), date);

    let date = manga_parser::util::try_parse_date("Release 2 month ago");
    compare_without_time(&now.checked_sub_months(Months::new(2)).unwrap(), date);

    let date = manga_parser::util::try_parse_date("2 years");
    compare_without_time(&now.checked_sub_months(Months::new(24)).unwrap(), date);
}

fn compare_without_time(expected: &DateTime<Utc>, actual: Option<DateTime<Utc>>) {
    assert_eq!(
        expected.date_naive(),
        actual.unwrap().date_naive(),
        "Dates are not the same"
    );
}
