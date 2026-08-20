use chrono::Utc;
use fcm::message::{Message, Notification, Target};
use fcm::FcmClient;
use manga_parser::Url;
use migration::{Expr, JoinType};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect, RelationTrait};
use serde_json::json;
use tokio::time::{self, Duration};

use crate::data;
use crate::service::v1::manga::{index_manga, save_manga};

pub async fn watch_updates(db: &DatabaseConnection) {
    let interval_ms: u64 = std::env::var("MANGA_AUTO_UPDATE_INTERVAL_MS")
        .unwrap_or("14400000".to_string())
        .parse()
        .unwrap_or(14400000);

    let mut interval = time::interval(Duration::from_millis(interval_ms));

    loop {
        interval.tick().await;

        let update_list = collect_priority_manga(db).await;

        info!("[Auto Update] Found {} manga that should be updated", update_list.len());
        for manga in update_list {
            info!("[Auto Update] Automatically updating {}", manga.url);
            let url = Url::parse(&manga.url);
            match url {
                Ok(url) => {
                    let saved = save_manga(db, None, Some(manga.id), url).await;
                    match saved {
                        Ok(saved) => {
                            if saved.count_chapters != manga.count_chapters {
                                let readers = get_readers(db, manga.id).await;
                                let ids: Vec<String> = readers.into_iter().flat_map(|user| user.device_ids).collect();

                                info!("[Auto Update] Successfully updated {}", saved.url.to_string());
                                if !ids.is_empty() {
                                    send_notification(&manga, ids.as_slice()).await;
                                }

                                // Eager prefetch: this manga already has
                                // active readers (same scope as the FCM
                                // notification above), so warm the cache for
                                // just the newly-discovered chapters instead
                                // of leaving the first reader to eat a slow
                                // chapter-by-chapter download.
                                if saved.count_chapters > manga.count_chapters {
                                    let new_count = (saved.count_chapters - manga.count_chapters) as u64;
                                    prefetch_new_chapters(db, manga.id, new_count).await;
                                }
                            }
                        }
                        Err(e) => {
                            error!("[Auto Update] URL Failed to Parse: {:#?}", e);
                        }
                    }
                }
                Err(e) => error!("[Auto Update] URL Failed to Parse: {:#?}", e),
            }
        }
    }
}

async fn collect_priority_manga(db: &DatabaseConnection) -> Vec<data::manga::Full> {
    // select 10 manga sorted by highest reading count
    // select only ones that havent been reloaded for at least "MANGA_AUTO_UPDATE_MIN_INTERVAL" milliseconds
    let min_interval: i64 = std::env::var("MANGA_AUTO_UPDATE_MIN_INTERVAL_MS")
        .unwrap_or("28800000".to_string())
        .parse()
        .unwrap_or(28800000);
    let limit: u64 = std::env::var("MANGA_AUTO_UPDATE_MAX")
        .unwrap_or("10".to_string())
        .parse()
        .unwrap_or(10);

    use entity::{manga, reading};

    let date_time = Utc::now().checked_sub_signed(chrono::Duration::milliseconds(min_interval));

    index_manga(None)
        .join(JoinType::LeftJoin, entity::reading::Relation::Manga.def().rev())
        .column_as(reading::Column::MangaId.count(), "count_reading")
        .group_by(manga::Column::Id)
        .filter(manga::Column::UpdatedAt.lte(date_time))
        .limit(limit)
        .order_by_desc(reading::Column::MangaId.count())
        .order_by_asc(manga::Column::UpdatedAt)
        .having(Expr::cust("COUNT(reading.manga_id) > 0"))
        .into_model::<data::manga::Full>()
        .all(db)
        .await
        .unwrap()
}

/// Spawn `prefetch_chapter` (fire-and-forget) for just the `count` most
/// recently inserted chapters of `manga_id` -- the ones `save_manga` just
/// added (existing chapters are always skipped via the `url` unique
/// constraint's `on_conflict do_nothing`, so the highest-`id` rows for this
/// manga are exactly the new ones).
async fn prefetch_new_chapters(db: &DatabaseConnection, manga_id: i32, count: u64) {
    let new_chapters = entity::chapter::Entity::find()
        .filter(entity::chapter::Column::MangaId.eq(manga_id))
        .order_by_desc(entity::chapter::Column::Id)
        .limit(count)
        .all(db)
        .await
        .unwrap_or_default();

    for chapter in new_chapters {
        info!(
            "[Auto Update] Prefetching images for new chapter {} [{}]",
            chapter.id, chapter.url
        );
        let db = db.clone();
        tokio::spawn(async move {
            crate::util::chapter_images::prefetch_chapter(db, chapter).await;
        });
    }
}

async fn get_readers(db: &DatabaseConnection, manga_id: i32) -> Vec<entity::user::Model> {
    entity::manga::Entity::find_by_id(manga_id)
        .find_with_related(entity::user::Entity)
        .all(db)
        .await
        .unwrap()
        .into_iter()
        .flat_map(|(_, users)| users)
        .collect()
}

async fn send_notification(manga: &data::manga::Full, ids: &[String]) {
    let client = FcmClient::builder()
        .service_account_key_json_path("manga-reader-5c535-148af5dd8096.json")
        .build()
        .await;

    if let Ok(client) = client {
        info!("Sending notifications to {} users", ids.len());

        let data = json!({ "manga_id": manga.id });

        for target in ids.to_owned() {
            let message = Message {
                data: Some(data.clone()),
                notification: Some(Notification {
                    title: Some("Manga Updated!".to_string()),
                    body: Some(manga.title.to_string()),
                    image: None,
                }),
                target: Target::Token(target),
                android: None,
                webpush: None,
                apns: None,
                fcm_options: None,
            };

            let response = client.send(message).await.unwrap();
            info!("Sent: {:?}", response);
        }
    } else {
        info!("FCM Error: {:#?}", client.err().unwrap());
    }
}
