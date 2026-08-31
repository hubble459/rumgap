use sea_orm_migration::prelude::*;

use crate::extension::timestamps::TimestampExt;
use crate::m20221130_215749_create_chapter::Chapter;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ChapterImage::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(ChapterImage::ChapterId).integer().not_null())
                    .col(ColumnDef::new(ChapterImage::PageIndex).integer().not_null())
                    .primary_key(
                        Index::create()
                            .col(ChapterImage::ChapterId)
                            .col(ChapterImage::PageIndex),
                    )
                    .col(
                        ColumnDef::new(ChapterImage::Status)
                            .string_len(255)
                            .not_null()
                            .default("pending"),
                    )
                    .col(ColumnDef::new(ChapterImage::SourceUrl).string_len(1023).not_null())
                    .col(ColumnDef::new(ChapterImage::StorageKey).string_len(511))
                    .col(ColumnDef::new(ChapterImage::ContentType).string_len(255))
                    .col(ColumnDef::new(ChapterImage::ByteSize).big_integer())
                    .col(ColumnDef::new(ChapterImage::Checksum).string_len(64))
                    .col(ColumnDef::new(ChapterImage::Error).text())
                    .col(ColumnDef::new(ChapterImage::Attempts).integer().not_null().default(0))
                    .foreign_key(
                        ForeignKey::create()
                            .from(ChapterImage::Table, ChapterImage::ChapterId)
                            .to(Chapter::Table, Chapter::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .take(),
            )
            .await?;

        // `updated_at` (from the shared timestamps trigger) doubles as
        // "last attempt time" for the retry-cooldown logic in
        // `util::chapter_images` -- no separate `last_attempt_at` column
        // needed since every attempt writes a status change to the row.
        manager.timestamps(ChapterImage::Table).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ChapterImage::Table).take())
            .await
    }
}

/// Learn more at https://docs.rs/sea-query#iden
#[derive(Iden)]
pub(crate) enum ChapterImage {
    Table,
    ChapterId,
    PageIndex,
    Status,
    SourceUrl,
    StorageKey,
    ContentType,
    ByteSize,
    Checksum,
    Error,
    Attempts,
    Width,
    Height,
}
