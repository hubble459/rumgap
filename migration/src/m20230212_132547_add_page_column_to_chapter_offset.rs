use sea_orm_migration::prelude::*;

use crate::m20230206_144400_create_chapter_offset::ChapterOffset;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(Iden)]
enum ChapterOffsetWithOffsetPage {
    Page,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ChapterOffset::Table)
                    .add_column_if_not_exists(
                        ColumnDef::new(ChapterOffsetWithOffsetPage::Page)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .take(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ChapterOffset::Table)
                    .drop_column(ChapterOffsetWithOffsetPage::Page)
                    .take(),
            )
            .await
    }
}
