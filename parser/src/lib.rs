pub mod model;
pub mod parser;
pub mod plugin;
pub use reqwest::Url;
#[cfg(test)]
mod tests {
    use reqwest::Url;

    use crate::parser::{MangaParser, Parser};

    #[tokio::test]
    async fn main_parser() {
        let parser = MangaParser::new();

        let result = parser.images(Url::parse("https://isekaiscanmanga.com/manga/cancel-this-wish/chapter-53/").unwrap()).await;
        let result = result.unwrap();
        println!("{:#?}", result);
        assert!(!result.is_empty());

        // let url = Url::parse("https://mangadex.org/title/28c77530-dfa1-4b05-8ec3-998960ba24d4/otomege-sekai-wa-mob-ni-kibishii-sekai-desu").unwrap();

        // let manga = parser.manga(url).await.unwrap();

        // assert!(!manga.title.is_empty());
        // assert!(!manga.description.is_empty());

        // println!("{:#?}", manga);
    }
}
