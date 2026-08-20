//! Throwaway helper (Phase 2 validation only, safe to delete) -- scrapes a
//! real manga + its first chapter's image list via manga_parser directly,
//! so we can seed a test DB with real chapter/image URLs to validate the
//! download pipeline end-to-end.
use manga_parser::scraper::scraper_manager::ScraperManager;
use manga_parser::scraper::MangaScraper;
use manga_parser::Url;

#[tokio::main]
async fn main() {
    let manager = ScraperManager::default();
    let manga_url = std::env::args().nth(1).expect("usage: fetch_test_chapter <manga_url>");
    let url = Url::parse(&manga_url).unwrap();

    let manga = manager.manga(&url).await.expect("failed to scrape manga");
    println!("Title: {}", manga.title);
    println!("Chapters: {}", manga.chapters.len());

    let first = manga.chapters.first().expect("no chapters found");
    println!("First chapter url: {}", first.url);
    println!("First chapter number: {}", first.number);

    let images = manager.chapter_images(&first.url).await.expect("failed to scrape images");
    println!("Images: {}", images.len());
    for image in &images {
        println!("  {}", image);
    }
}
