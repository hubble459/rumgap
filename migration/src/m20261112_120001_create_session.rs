use sea_orm_migration::prelude::*;

use crate::m20221127_174334_create_user::User;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // One row per login. A token is only valid while its row exists and hasn't
        // expired, so deleting rows is how a session (or all of a user's) gets revoked.
        manager
            .create_table(
                Table::create()
                    .table(Session::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Session::Id).string_len(64).not_null().primary_key())
                    .col(ColumnDef::new(Session::UserId).integer().not_null())
                    .col(ColumnDef::new(Session::UserAgent).string_len(512))
                    .col(
                        ColumnDef::new(Session::CreatedAt)
                            .timestamp()
                            .extra("DEFAULT NOW()".to_owned())
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Session::LastUsedAt)
                            .timestamp()
                            .extra("DEFAULT NOW()".to_owned())
                            .not_null(),
                    )
                    .col(ColumnDef::new(Session::ExpiresAt).timestamp().not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .from(Session::Table, Session::UserId)
                            .to(User::Table, User::Id)
                            .on_update(ForeignKeyAction::Cascade)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .take(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_session_user_id")
                    .table(Session::Table)
                    .col(Session::UserId)
                    .take(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Session::Table).take()).await
    }
}

#[derive(Iden)]
pub(crate) enum Session {
    Table,
    Id,
    UserId,
    UserAgent,
    CreatedAt,
    LastUsedAt,
    ExpiresAt,
}
