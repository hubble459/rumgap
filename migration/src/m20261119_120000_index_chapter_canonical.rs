use sea_orm_migration::prelude::*;

use crate::m20221130_215749_create_chapter::Chapter;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create()
                    .name("idx_chapter_canonical_chapter_id")
                    .table(Chapter::Table)
                    .col(Chapter::CanonicalChapterId)
                    .col(Chapter::MangaSourceId)
                    .take(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_chapter_manga_source_id_number_id")
                    .table(Chapter::Table)
                    .col(Chapter::MangaSourceId)
                    .col(Chapter::Number)
                    .col(Chapter::Id)
                    .take(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_chapter_manga_source_id_number_id")
                    .table(Chapter::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_index(
                Index::drop()
                    .name("idx_chapter_canonical_chapter_id")
                    .table(Chapter::Table)
                    .to_owned(),
            )
            .await
    }
}
