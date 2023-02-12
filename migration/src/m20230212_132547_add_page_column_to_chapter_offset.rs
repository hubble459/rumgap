use sea_orm_migration::prelude::*;

use crate::m20230206_144400_create_chapter_offset::ChapterOffset;

#[derive(DeriveMigrationName)]
pub struct Migration;


#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let offset_page: Alias = Alias::new("page");

        manager
            .alter_table(
                Table::alter()
                    .table(ChapterOffset::Table)
                    .add_column_if_not_exists(
                        ColumnDef::new(offset_page).integer().not_null().default(0),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let offset_page: Alias = Alias::new("page");

        manager
            .alter_table(
                Table::alter()
                    .table(ChapterOffset::Table)
                    .drop_column(offset_page)
                    .to_owned(),
            )
            .await
    }
}
