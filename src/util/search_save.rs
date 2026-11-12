//! Saves online search results as manga in the background, so the catalog (and with it
//! the "for you" candidates) grows with every search. One worker drains a deduplicated
//! queue, one manga at a time with a delay in between, to go easy on the scraped sites and
//! the host. `create_manga` attaches a result to an existing manga when its titles match.

use std::collections::{HashSet, VecDeque};
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use lazy_static::lazy_static;
use log::{info, warn};
use manga_parser::Url;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter};

use crate::service::v1::manga::create_manga;

const MAX_QUEUED: usize = 500;

#[derive(Default)]
struct Queue {
    urls: VecDeque<Url>,
    queued: HashSet<String>,
    running: bool,
}

lazy_static! {
    static ref QUEUE: StdMutex<Queue> = StdMutex::new(Queue::default());
}

fn enabled() -> bool {
    std::env::var("SEARCH_AUTO_SAVE").map_or(true, |value| value != "false")
}

fn delay() -> Duration {
    Duration::from_millis(
        std::env::var("SEARCH_AUTO_SAVE_DELAY_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(2000),
    )
}

/// Queues `urls` for saving; urls already queued are skipped. Starts the worker if it
/// isn't running.
pub fn enqueue(db: &DatabaseConnection, urls: impl IntoIterator<Item = Url>) {
    if !enabled() {
        return;
    }

    let mut queue = QUEUE.lock().unwrap();
    for url in urls {
        if queue.urls.len() >= MAX_QUEUED {
            break;
        }
        if queue.queued.insert(url.to_string()) {
            queue.urls.push_back(url);
        }
    }

    if !queue.running && !queue.urls.is_empty() {
        queue.running = true;
        let db = db.clone();
        tokio::spawn(async move { work(db).await });
    }
}

fn next() -> Option<Url> {
    let mut queue = QUEUE.lock().unwrap();
    let url = queue.urls.pop_front();
    match &url {
        Some(url) => {
            queue.queued.remove(&url.to_string());
        }
        None => queue.running = false,
    }
    url
}

async fn work(db: DatabaseConnection) {
    while let Some(url) = next() {
        match entity::manga_source::Entity::find()
            .filter(entity::manga_source::Column::Url.eq(url.to_string()))
            .count(&db)
            .await
        {
            Ok(0) => {}
            Ok(_) => continue,
            Err(e) => {
                warn!("[Search Save] Couldn't check [{url}]: {e}");
                continue;
            }
        }

        let task = {
            let (db, url) = (db.clone(), url.clone());
            tokio::spawn(async move { create_manga(&db, url).await })
        };
        match task.await {
            Ok(Ok(manga_id)) => info!("[Search Save] Saved [{url}] as manga {manga_id}"),
            Ok(Err(e)) => warn!("[Search Save] Couldn't save [{url}]: {}", e.message()),
            Err(e) => warn!("[Search Save] Saving [{url}] panicked: {e}"),
        }
        tokio::time::sleep(delay()).await;
    }
}
