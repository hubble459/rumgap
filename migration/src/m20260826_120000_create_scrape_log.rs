use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ScrapeLog::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ScrapeLog::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ScrapeLog::Hostname).string_len(255).not_null())
                    .col(ColumnDef::new(ScrapeLog::Operation).string_len(64).not_null())
                    .col(ColumnDef::new(ScrapeLog::Url).string_len(1023).not_null())
                    // Deliberately no FK to manga/manga_source -- a log row must outlive the
                    // row it was about (a manga/source can be deleted long before its scrape
                    // history should be), so these are just plain nullable references.
                    .col(ColumnDef::new(ScrapeLog::MangaId).integer())
                    .col(ColumnDef::new(ScrapeLog::MangaSourceId).integer())
                    .col(ColumnDef::new(ScrapeLog::Success).boolean().not_null())
                    .col(ColumnDef::new(ScrapeLog::ErrorType).string_len(64))
                    .col(ColumnDef::new(ScrapeLog::ErrorMessage).text())
                    .col(ColumnDef::new(ScrapeLog::DurationMs).integer().not_null())
                    .col(
                        ColumnDef::new(ScrapeLog::CreatedAt)
                            .timestamp()
                            .extra("DEFAULT NOW()".to_owned())
                            .not_null(),
                    )
                    .take(),
            )
            .await?;

        // Backs both the admin status page's per-hostname grouping and the periodic prune
        // (which deletes by success + age).
        manager
            .create_index(
                Index::create()
                    .name("idx_scrape_log_hostname_created_at")
                    .table(ScrapeLog::Table)
                    .col(ScrapeLog::Hostname)
                    .col(ScrapeLog::CreatedAt)
                    .take(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_scrape_log_success_created_at")
                    .table(ScrapeLog::Table)
                    .col(ScrapeLog::Success)
                    .col(ScrapeLog::CreatedAt)
                    .take(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(ScrapeLog::Table).take()).await
    }
}

/// Learn more at https://docs.rs/sea-query#iden
#[derive(Iden)]
pub(crate) enum ScrapeLog {
    Table,
    Id,
    Hostname,
    Operation,
    Url,
    MangaId,
    MangaSourceId,
    Success,
    ErrorType,
    ErrorMessage,
    DurationMs,
    CreatedAt,
}
