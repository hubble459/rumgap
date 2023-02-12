use sea_orm_migration::prelude::*;

use crate::m20221127_174334_create_user::User;
use crate::m20221130_215749_create_chapter::Chapter;
use crate::trigger;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ChapterOffset::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(ChapterOffset::UserId).integer().not_null())
                    .col(
                        ColumnDef::new(ChapterOffset::ChapterId)
                            .integer()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(ChapterOffset::UserId)
                            .col(ChapterOffset::ChapterId),
                    )
                    .col(
                        ColumnDef::new(ChapterOffset::Offset)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(ChapterOffset::Table, ChapterOffset::ChapterId)
                            .to(Chapter::Table, Chapter::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(ChapterOffset::Table, ChapterOffset::UserId)
                            .to(User::Table, User::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        trigger::add_date_triggers(manager, ChapterOffset::Table.to_string()).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        trigger::drop_date_triggers(manager, ChapterOffset::Table.to_string()).await?;

        manager
            .drop_table(Table::drop().table(ChapterOffset::Table).to_owned())
            .await
    }
}

/// Learn more at https://docs.rs/sea-query#iden
#[derive(Iden)]
pub(crate) enum ChapterOffset {
    Table,
    UserId,
    ChapterId,
    Offset,
}
