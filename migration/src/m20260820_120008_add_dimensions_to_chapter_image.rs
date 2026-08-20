use sea_orm_migration::prelude::*;

use crate::m20260820_120006_create_chapter_image::ChapterImage;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Read straight from the header bytes at download time (util::image_dimensions) -
        // no image-decoding crate needed. NULL for pages that failed to download, or whose
        // format isn't PNG/JPEG (the two formats that header-only parsing covers).
        manager
            .alter_table(
                Table::alter()
                    .table(ChapterImage::Table)
                    .add_column(ColumnDef::new(ChapterImage::Width).integer())
                    .add_column(ColumnDef::new(ChapterImage::Height).integer())
                    .take(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ChapterImage::Table)
                    .drop_column(ChapterImage::Width)
                    .drop_column(ChapterImage::Height)
                    .take(),
            )
            .await
    }
}
