use sea_orm_migration::prelude::*;

use crate::extension::timestamps::TimestampExt;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Manga::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Manga::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Manga::Url).string_len(511).not_null().unique_key())
                    .col(ColumnDef::new(Manga::Title).string_len(511).not_null())
                    .col(ColumnDef::new(Manga::Description).string().not_null())
                    .col(ColumnDef::new(Manga::Cover).string_len(511))
                    .col(ColumnDef::new(Manga::IsOngoing).boolean().not_null())
                    .col(
                        ColumnDef::new(Manga::Genres)
                            .array(ColumnType::String(StringLen::N(255)))
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Manga::Authors)
                            .array(ColumnType::String(StringLen::N(255)))
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Manga::AltTitles)
                            .array(ColumnType::String(StringLen::N(255)))
                            .not_null(),
                    )
                    .take(),
            )
            .await?;

        manager.timestamps(Manga::Table).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Manga::Table).take()).await
    }
}

/// Learn more at https://docs.rs/sea-query#iden
#[derive(Iden)]
pub(crate) enum Manga {
    Table,
    Id,
    Url,
    Title,
    Description,
    Cover,
    IsOngoing,
    Genres,
    Authors,
    AltTitles,
    Status,
}
