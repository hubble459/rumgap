use std::collections::HashMap;

use chrono::Utc;
use fcm::{Client, MessageBuilder, NotificationBuilder};
use manga_parser::Url;
use migration::{Expr, JoinType};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    RelationTrait,
};
use tokio::time::{self, Duration};

use crate::{
    data,
    service::manga::{index_manga, save_manga},
};

pub async fn watch_updates(db: &DatabaseConnection) {
    let interval_ms: u64 = std::env::var("MANGA_UPDATE_INTERVAL_MS")
        .unwrap_or("3600000".to_string())
        .parse()
        .unwrap_or(3600000);

    let mut interval = time::interval(Duration::from_millis(interval_ms));

    loop {
        interval.tick().await;

        let update_list = collect_priority_manga(db).await;

        info!(
            "[Auto Update] Found {} manga that should be updated",
            update_list.len()
        );
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
                                let ids: Vec<String> = readers
                                    .into_iter()
                                    .flat_map(|user| user.device_ids)
                                    .collect();

                                info!(
                                    "[Auto Update] Successfully updated {}",
                                    saved.url.to_string()
                                );
                                if !ids.is_empty() {
                                    send_notification(&manga, ids.as_slice()).await;
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
    // select only ones that havent been reloaded for at least "MANGA_UPDATE_INTERVAL_MS" * 2 milliseconds
    let interval_ms: i64 = std::env::var("MANGA_UPDATE_INTERVAL_MS")
        .unwrap_or("3600000".to_string())
        .parse()
        .unwrap_or(3600000);
    let limit: u64 = std::env::var("MANGA_AUTO_UPDATE_MAX")
        .unwrap_or("10".to_string())
        .parse()
        .unwrap_or(10);

    let interval_ms = interval_ms * 2;

    use entity::{manga, reading};

    let date_time = Utc::now().checked_sub_signed(chrono::Duration::milliseconds(interval_ms));

    let priority = index_manga(None)
        .join(
            JoinType::LeftJoin,
            entity::reading::Relation::Manga.def().rev(),
        )
        .column_as(reading::Column::MangaId.count(), "count_reading")
        .group_by(manga::Column::Id)
        .filter(manga::Column::UpdatedAt.lte(date_time))
        .limit(limit)
        .order_by_desc(reading::Column::MangaId.count())
        .having(Expr::cust("COUNT(reading.manga_id) > 0"))
        .into_model::<data::manga::Full>()
        .all(db)
        .await
        .unwrap();

    priority
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
    info!("Sending notifications to {} users", ids.len());

    let notification_tag = manga.id.to_string();

    let client = Client::new();
    let fcm_token = dotenvy::var("FCM_LEGACY_API_KEY").expect("Missing FCM token");

    let mut notification_builder = NotificationBuilder::new();
    notification_builder.title("Manga Updated!");
    notification_builder.body(&manga.title);
    notification_builder.tag(&notification_tag);
    notification_builder.click_action("MANGA_UPDATED");

    notification_builder.icon("https://cdn.discordapp.com/attachments/1013449250102857729/1076924437280067705/v2-84ce3eaa59c7a6f6fd8b8e23c7431c48_b.jpg");

    let mut builder = MessageBuilder::new_multi(&fcm_token, ids);
    builder.notification(notification_builder.finalize());
    let mut data = HashMap::new();
    data.insert("manga_id", manga.id.to_string());
    builder.data(&data).unwrap();

    let response = client.send(builder.finalize()).await.unwrap();
    println!("Sent: {:?}", response);
}
