use sea_orm_migration::prelude::*;

use crate::m20221130_215742_create_manga::Manga;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Manga::Table)
                    .add_column_if_not_exists(
                        ColumnDef::new(Manga::Status)
                            .string_len(255)
                            .default("Ongoing")
                            .not_null(),
                    )
                    .take(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(Table::alter().table(Manga::Table).drop_column(Manga::Status).take())
            .await
    }
}
