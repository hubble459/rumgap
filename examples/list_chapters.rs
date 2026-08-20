//! Throwaway helper (Phase 2 validation only, safe to delete) -- lists a
//! manga's chapter URLs so we can seed a handful of real chapters into the
//! test DB for BackfillImages validation.
use manga_parser::scraper::scraper_manager::ScraperManager;
use manga_parser::scraper::MangaScraper;
use manga_parser::Url;

#[tokio::main]
async fn main() {
    let manager = ScraperManager::default();
    let manga_url = std::env::args().nth(1).expect("usage: list_chapters <manga_url>");
    let count: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    let url = Url::parse(&manga_url).unwrap();

    let manga = manager.manga(&url).await.expect("failed to scrape manga");
    for chapter in manga.chapters.iter().take(count) {
        println!("{}\t{}\t{}", chapter.number, chapter.title, chapter.url);
    }
}
