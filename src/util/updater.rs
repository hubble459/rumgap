use chrono::Utc;
use manga_parser::Url;
use migration::{JoinType, Expr};
use sea_orm::{DatabaseConnection, EntityTrait, QuerySelect, RelationTrait, ColumnTrait, QueryFilter, QueryOrder};
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

        info!("[Auto Update] Found {} manga that should be updated", update_list.len());
        for (id, url) in update_list {
            info!("[Auto Update] Automatically updating {url}");
            let url = Url::parse(&url);
            match url {
                Ok(url) => {
                    let saved = save_manga(db, None, Some(id), url).await;
                    match saved {
                        Ok(saved) => {
                            info!("[Auto Update] Successfully updated {}", saved.url.to_string());
                        },
                        Err(e) => {
                            error!("[Auto Update] URL Failed to Parse: {:#?}", e);
                        },
                    }
                },
                Err(e) => error!("[Auto Update] URL Failed to Parse: {:#?}", e),
            }
            
        }
    }
}

async fn collect_priority_manga(db: &DatabaseConnection) -> Vec<(i32, String)> {
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

    let priority: Vec<(i32, String, i64)> = manga::Entity::find()
        .join(JoinType::LeftJoin, reading::Relation::Manga.def().rev())
        .select_only()
        .column(manga::Column::Id)
        .column(manga::Column::Url)
        .column_as(reading::Column::MangaId.count(), "count_reading")
        .group_by(manga::Column::Id)
        .filter(manga::Column::UpdatedAt.lte(date_time))
        .limit(limit)
        .order_by_desc(Expr::cust("2"))
        .into_tuple()
        .all(db)
        .await
        .unwrap();

    priority.into_iter().map(|a| (a.0, a.1)).collect()
}