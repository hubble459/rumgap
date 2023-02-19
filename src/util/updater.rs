use chrono::Utc;
use fcm::{Client, MessageBuilder, NotificationBuilder};
use manga_parser::Url;
use migration::Expr;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use tokio::time::{self, Duration};

use crate::service::manga::save_manga;

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
        for (manga, users) in update_list {
            info!("[Auto Update] Automatically updating {}", manga.url);
            let url = Url::parse(&manga.url);
            match url {
                Ok(url) => {
                    let saved = save_manga(db, None, Some(manga.id), url).await;
                    match saved {
                        Ok(saved) => {
                            let ids: Vec<String> =
                                users.into_iter().flat_map(|user| user.device_ids).collect();

                            info!(
                                "[Auto Update] Successfully updated {}",
                                saved.url.to_string()
                            );
                            if !ids.is_empty() {
                                send_notification(&manga, ids.as_slice()).await;
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

async fn collect_priority_manga(
    db: &DatabaseConnection,
) -> Vec<(entity::manga::Model, Vec<entity::user::Model>)> {
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

    use entity::{manga, reading, user};

    let date_time = Utc::now().checked_sub_signed(chrono::Duration::milliseconds(interval_ms));

    let priority = manga::Entity::find()
        .find_with_related(entity::user::Entity)
        .column_as(reading::Column::MangaId.count(), "count_reading")
        .group_by(manga::Column::Id)
        .group_by(user::Column::Id)
        .filter(manga::Column::UpdatedAt.lte(date_time))
        .limit(limit)
        .order_by_desc(reading::Column::MangaId.count())
        .having(Expr::cust("COUNT(reading.manga_id) > 0"))
        .all(db)
        .await
        .unwrap();

    priority
}

async fn send_notification(manga: &entity::manga::Model, ids: &[String]) {
    info!("Sending notifications to {} users", ids.len());

    let notification_tag = manga.id.to_string();

    let client = Client::new();
    let fcm_token = dotenvy::var("FCM_LEGACY_API_KEY").expect("Missing FCM token");

    let mut notification_builder = NotificationBuilder::new();
    notification_builder.title("Manga Updated!");
    notification_builder.body(&manga.title);
    notification_builder.tag(&notification_tag);

    notification_builder.icon("https://cdn.discordapp.com/attachments/1013449250102857729/1076924437280067705/v2-84ce3eaa59c7a6f6fd8b8e23c7431c48_b.jpg");

    let mut builder = MessageBuilder::new_multi(&fcm_token, ids);
    builder.notification(notification_builder.finalize());

    let response = client.send(builder.finalize()).await.unwrap();
    println!("Sent: {:?}", response);
}
