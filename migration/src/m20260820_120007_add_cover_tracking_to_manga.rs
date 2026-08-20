use sea_orm_migration::prelude::*;

use crate::m20221130_215742_create_manga::Manga;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // `manga.cover` becomes the raw scraped URL only (set only by primary-source
        // refreshes, same rule as today) -- the local-hosting tracking columns below carry
        // the download/cache state, same convention as `chapter_image`.
        manager
            .alter_table(
                Table::alter()
                    .table(Manga::Table)
                    .rename_column(Manga::Cover, Manga::CoverSourceUrl)
                    .take(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Manga::Table)
                    .add_column_if_not_exists(
                        ColumnDef::new(Manga::CoverStatus)
                            .string_len(255)
                            .not_null()
                            .default("pending"),
                    )
                    .add_column_if_not_exists(ColumnDef::new(Manga::CoverStorageKey).string_len(511))
                    .add_column_if_not_exists(ColumnDef::new(Manga::CoverContentType).string_len(255))
                    .add_column_if_not_exists(ColumnDef::new(Manga::CoverChecksum).string_len(64))
                    .add_column_if_not_exists(ColumnDef::new(Manga::CoverAttempts).integer().not_null().default(0))
                    .take(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Manga::Table)
                    .drop_column(Manga::CoverStatus)
                    .drop_column(Manga::CoverStorageKey)
                    .drop_column(Manga::CoverContentType)
                    .drop_column(Manga::CoverChecksum)
                    .drop_column(Manga::CoverAttempts)
                    .take(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Manga::Table)
                    .rename_column(Manga::CoverSourceUrl, Manga::Cover)
                    .take(),
            )
            .await
    }
}
