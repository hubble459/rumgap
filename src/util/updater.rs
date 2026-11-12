use chrono::Utc;
use fcm::message::{Message, Notification, Target, WebpushConfig};
use fcm::FcmClient;
use migration::{Expr, JoinType};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect, RelationTrait};
use serde_json::json;
use tokio::time::{self, Duration};

use crate::data;
use crate::service::v1::manga::{get_manga_by_id, index_manga, refresh_manga_source};

pub async fn watch_updates(db: &DatabaseConnection) {
    let interval_ms: u64 = std::env::var("MANGA_AUTO_UPDATE_INTERVAL_MS")
        .unwrap_or("14400000".to_string())
        .parse()
        .unwrap_or(14400000);

    let mut interval = time::interval(Duration::from_millis(interval_ms));

    loop {
        interval.tick().await;

        crate::util::scrape_log::prune(db).await;
        crate::util::session::prune(db).await;

        let update_list = collect_priority_manga(db).await;

        info!("[Auto Update] Found {} manga that should be updated", update_list.len());
        for manga in update_list {
            let primary_source = entity::manga_source::Entity::find()
                .filter(entity::manga_source::Column::MangaId.eq(manga.id))
                .filter(entity::manga_source::Column::IsPrimary.eq(true))
                .one(db)
                .await
                .unwrap();

            let Some(primary_source) = primary_source else {
                error!("[Auto Update] Manga {} has no primary source, skipping", manga.id);
                continue;
            };

            info!("[Auto Update] Automatically updating {}", primary_source.url);

            match refresh_manga_source(db, primary_source.id, false).await {
                Ok(manga_id) => match get_manga_by_id(db, None, manga_id).await {
                    Ok(saved) => {
                        if saved.count_chapters != manga.count_chapters {
                            let readers = get_readers(db, manga.id).await;
                            let ids: Vec<String> = readers.into_iter().flat_map(|user| user.device_ids).collect();

                            info!("[Auto Update] Successfully updated {}", primary_source.url);
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
                                prefetch_new_chapters(db, primary_source.id, new_count).await;
                            }
                        }
                    }
                    Err(e) => {
                        error!("[Auto Update] Failed to re-fetch manga after update: {:#?}", e);
                    }
                },
                Err(e) => {
                    error!("[Auto Update] Failed to refresh source: {:#?}", e);
                }
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

    let stale_before = Utc::now().naive_utc() - chrono::Duration::milliseconds(min_interval);
    let failure_cutoff = Utc::now().naive_utc() - chrono::Duration::milliseconds(min_interval);

    index_manga(None)
        .join(JoinType::LeftJoin, entity::reading::Relation::Manga.def().rev())
        .column_as(reading::Column::MangaId.count(), "count_reading")
        .group_by(manga::Column::Id)
        // index_manga(None) joins each manga's primary source, the one refreshed here.
        .filter(
            sea_orm::Condition::any()
                .add(entity::manga_source::Column::DetailsUpdatedAt.is_null())
                .add(entity::manga_source::Column::DetailsUpdatedAt.lte(stale_before)),
        )
        .filter(Expr::cust_with_values(
            "NOT EXISTS (SELECT 1 FROM scrape_log WHERE scrape_log.manga_id = manga.id \
             AND scrape_log.operation = 'manga' AND NOT scrape_log.success AND scrape_log.created_at > $1)",
            [failure_cutoff],
        ))
        .limit(limit)
        .order_by_desc(reading::Column::MangaId.count())
        .order_by_asc(entity::manga_source::Column::DetailsUpdatedAt)
        .having(Expr::cust("COUNT(reading.manga_id) > 0"))
        .into_model::<data::manga::Full>()
        .all(db)
        .await
        .unwrap()
}

/// Spawn `prefetch_chapter` (fire-and-forget) for just the `count` most
/// recently inserted chapters of `manga_source_id` -- the ones
/// `refresh_manga_source` just added (existing chapters are always skipped
/// via the `url` unique constraint's `on_conflict do_nothing`, so the
/// highest-`id` rows for this source are exactly the new ones).
async fn prefetch_new_chapters(db: &DatabaseConnection, manga_source_id: i32, count: u64) {
    let new_chapters = entity::chapter::Entity::find()
        .filter(entity::chapter::Column::MangaSourceId.eq(manga_source_id))
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
    // Must belong to the same Firebase project the apps register their tokens
    // with (wuxia-6a4d2), otherwise FCM rejects every send with SenderIdMismatch.
    let key_path = std::env::var("FCM_SERVICE_ACCOUNT_KEY").unwrap_or_else(|_| "firebase-service-account.json".to_string());
    // Web pushes don't pick up an icon on their own -- the browser shows its default one.
    let web_icon = std::env::var("WEB_NOTIFICATION_ICON")
        .unwrap_or_else(|_| "https://manga.quentincorreia.nl/icons/Icon-192.png".to_string());

    let client = FcmClient::builder()
        .service_account_key_json_path(key_path)
        .build()
        .await;

    if let Ok(client) = client {
        info!("Sending notifications to {} users", ids.len());

        let data = json!({ "manga_id": manga.id });

        for target in ids.iter().cloned() {
            let message = Message {
                data: Some(data.clone()),
                notification: Some(Notification {
                    title: Some("Manga Updated!".to_string()),
                    body: manga.title.clone(),
                    image: None,
                }),
                target: Target::Token(target),
                android: None,
                webpush: Some(WebpushConfig {
                    notification: Some(json!({ "icon": web_icon })),
                    ..Default::default()
                }),
                apns: None,
                fcm_options: None,
            };

            match client.send(message).await {
                Ok(response) => info!("Sent: {:?}", response),
                Err(err) => error!("Failed to send notification: {:#?}", err),
            }
        }
    } else {
        info!("FCM Error: {:#?}", client.err().unwrap());
    }
}

fn source_details_interval_ms() -> u64 {
    std::env::var("SOURCE_DETAILS_INTERVAL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60_000)
}

/// Slowly scrapes sources whose details were never scraped since sources got their own
/// (until then they're a copy of the primary's), a few per tick. Sources that recently
/// failed to scrape are skipped until `AUTO_BACKFILL_FAILURE_COOLDOWN_MS` passes.
pub async fn watch_source_details(db: DatabaseConnection) {
    let mut interval = time::interval(Duration::from_millis(source_details_interval_ms()));
    let failure_cooldown_ms: i64 = std::env::var("AUTO_BACKFILL_FAILURE_COOLDOWN_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(86_400_000);

    loop {
        interval.tick().await;

        let failure_cutoff = Utc::now().naive_utc() - chrono::Duration::milliseconds(failure_cooldown_ms);
        let sources: Vec<i32> = match entity::manga_source::Entity::find()
            .filter(entity::manga_source::Column::DetailsUpdatedAt.is_null())
            .filter(Expr::cust_with_values(
                "NOT EXISTS (SELECT 1 FROM scrape_log WHERE scrape_log.url = manga_source.url \
                 AND scrape_log.operation = 'manga' AND NOT scrape_log.success AND scrape_log.created_at > $1)",
                [failure_cutoff],
            ))
            .select_only()
            .column(entity::manga_source::Column::Id)
            .order_by_asc(entity::manga_source::Column::Id)
            .limit(3)
            .into_tuple()
            .all(&db)
            .await
        {
            Ok(sources) => sources,
            Err(e) => {
                error!("[Source Details] Failed to list unscraped sources: {:#?}", e);
                continue;
            }
        };

        for source_id in sources {
            if let Err(e) = refresh_manga_source(&db, source_id, false).await {
                warn!("[Source Details] Failed to scrape source {}: {}", source_id, e.message());
            }
        }
    }
}
