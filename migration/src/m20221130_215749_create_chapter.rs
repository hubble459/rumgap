use sea_orm_migration::prelude::*;

use crate::{trigger, m20221130_215742_create_manga::Manga};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Chapter::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Chapter::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Chapter::MangaId).integer().not_null())
                    .col(
                        ColumnDef::new(Chapter::Url)
                            .string_len(511)
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(Chapter::Title).string_len(511).not_null())
                    .col(ColumnDef::new(Chapter::Number).float().not_null())
                    .col(ColumnDef::new(Chapter::Posted).timestamp_with_time_zone())
                    .foreign_key(
                        ForeignKey::create()
                            .from(Chapter::Table, Chapter::MangaId)
                            .to(Manga::Table, Manga::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        trigger::add_date_triggers(manager, Chapter::Table.to_string()).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        trigger::drop_date_triggers(manager, Chapter::Table.to_string()).await?;

        manager
            .drop_table(Table::drop().table(Chapter::Table).to_owned())
            .await
    }
}

/// Learn more at https://docs.rs/sea-query#iden
#[derive(Iden)]
pub(crate) enum Chapter {
    Table,
    Id,
    MangaId,
    Url,
    Title,
    Number,
    Posted,
}
