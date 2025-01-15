use sea_orm_migration::prelude::*;

use crate::extension::timestamps::TimestampExt;
use crate::m20221127_174334_create_user::User;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Friend::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Friend::UserId).integer().not_null())
                    .col(ColumnDef::new(Friend::FriendId).integer().not_null())
                    .primary_key(Index::create().col(Friend::UserId).col(Friend::FriendId))
                    .foreign_key(
                        ForeignKey::create()
                            .from(Friend::Table, Friend::UserId)
                            .to(User::Table, User::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(Friend::Table, Friend::FriendId)
                            .to(User::Table, User::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .take(),
            )
            .await?;

        manager.timestamps(Friend::Table).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Friend::Table).take()).await
    }
}

/// Learn more at https://docs.rs/sea-query#iden
#[derive(Iden)]
pub(crate) enum Friend {
    Table,
    UserId,
    FriendId,
}
