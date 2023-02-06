use sea_orm_migration::prelude::*;

use crate::trigger;

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
                    .col(ColumnDef::new(Manga::Genres).array(ColumnType::String(Some(255))).not_null())
                    .col(ColumnDef::new(Manga::Authors).array(ColumnType::String(Some(255))).not_null())
                    .col(ColumnDef::new(Manga::AltTitles).array(ColumnType::String(Some(255))).not_null())
                    .to_owned(),
            )
            .await?;

        trigger::add_date_triggers(manager, Manga::Table.to_string()).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        trigger::drop_date_triggers(manager, Manga::Table.to_string()).await?;

        manager
            .drop_table(Table::drop().table(Manga::Table).to_owned())
            .await
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
}
