pub mod model;
pub mod parser;
pub mod plugin;

#[cfg(test)]
mod tests {
    use crate::parser::{MangaParser, Parser};

    #[tokio::test]
    async fn main_parser() {
        let parser = MangaParser::new();

        let hostnames = parser.hostnames();
        assert_eq!(hostnames.len(), 2);

        let url = reqwest::Url::parse("https://mangadex.org/title/28c77530-dfa1-4b05-8ec3-998960ba24d4/otomege-sekai-wa-mob-ni-kibishii-sekai-desu").unwrap();

        let manga = parser.manga(url).await;

        println!("{:#?}", manga);
    }
}
