use sea_orm_migration::prelude::*;

use crate::extension::timestamps::TimestampExt;
use crate::m20221127_174334_create_user::User;
use crate::m20221130_215742_create_manga::Manga;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Reading::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Reading::UserId).integer().not_null())
                    .col(ColumnDef::new(Reading::MangaId).integer().not_null())
                    .primary_key(Index::create().col(Reading::UserId).col(Reading::MangaId))
                    .col(ColumnDef::new(Reading::Progress).integer().not_null().default(0))
                    .foreign_key(
                        ForeignKey::create()
                            .from(Reading::Table, Reading::MangaId)
                            .to(Manga::Table, Manga::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(Reading::Table, Reading::UserId)
                            .to(User::Table, User::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .take(),
            )
            .await?;

        manager.timestamps(Reading::Table).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Reading::Table).take()).await
    }
}

/// Learn more at https://docs.rs/sea-query#iden
#[derive(Iden)]
pub(crate) enum Reading {
    Table,
    MangaId,
    UserId,
    Progress,
}
