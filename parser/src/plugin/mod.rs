use crate::parser::Parser;

use self::madara::Madara;
use self::mangadex::MangaDex;
use self::read_m::ReadM;

pub mod generic_query_parser;
mod madara;
mod mangadex;
mod read_m;

pub fn plugins() -> Vec<Box<dyn Parser + Send + Sync>> {
    vec![Box::new(MangaDex::new()), Box::new(Madara::new()), Box::new(ReadM::new())]
}
