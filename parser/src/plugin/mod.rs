use crate::parser::Parser;

mod mangadex;

pub fn plugins() -> Vec<Box<dyn Parser + Send + Sync>> {
    vec![Box::new(mangadex::MangaDex::new())]
}
