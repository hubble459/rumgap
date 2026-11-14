use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::Statement;

#[derive(DeriveMigrationName)]
pub struct Migration;

async fn execute(manager: &SchemaManager<'_>, sql: &str) -> Result<(), DbErr> {
    manager
        .get_connection()
        .execute_raw(Statement::from_string(manager.get_database_backend(), sql.to_owned()))
        .await
        .map(|_| ())
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    // Where a manga comes from: manga, manhwa, manhua or unknown. Detected from its sources
    // (`manga_source.original_language` where the site says, else genre tags, else the
    // script of its titles) unless a mod set it by hand.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        execute(manager, "ALTER TABLE manga_source ADD COLUMN original_language varchar").await?;
        execute(
            manager,
            "ALTER TABLE manga ADD COLUMN origin varchar NOT NULL DEFAULT 'unknown', \
             ADD COLUMN origin_set_by_hand boolean NOT NULL DEFAULT false",
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        execute(
            manager,
            "ALTER TABLE manga DROP COLUMN origin, DROP COLUMN origin_set_by_hand",
        )
        .await?;
        execute(manager, "ALTER TABLE manga_source DROP COLUMN original_language").await
    }
}
