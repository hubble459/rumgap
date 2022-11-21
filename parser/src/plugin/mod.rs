use crate::parser::Parser;

use self::madara::Madara;
use self::manga_347::Manga347;
use self::manga_kakalot::MangaKakalot;
use self::mangadex::MangaDex;
use self::read_m::ReadM;
use self::reaper_scans::ReaperScans;

pub mod generic_query_parser;
mod madara;
mod manga_347;
mod manga_kakalot;
mod mangadex;
mod read_m;
mod reaper_scans;

pub fn plugins() -> Vec<Box<dyn Parser + Send + Sync>> {
    vec![
        Box::new(Madara::new()),
        Box::new(Manga347::new()),
        Box::new(MangaDex::new()),
        Box::new(MangaKakalot::new()),
        Box::new(ReadM::new()),
        Box::new(ReaperScans::new()),
    ]
}
