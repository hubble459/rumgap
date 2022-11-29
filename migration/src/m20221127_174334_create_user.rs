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
                    .table(User::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(User::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(User::Permissions).small_integer().not_null().default(0))
                    .col(ColumnDef::new(User::Username).string_len(15).unique_key().not_null())
                    .col(ColumnDef::new(User::Email).string_len(255).unique_key().not_null())
                    .col(
                        ColumnDef::new(User::PasswordHash)
                            .string_len(255)
                            .not_null(),
                    )
                    .take(),
            )
            .await?;

        trigger::add_date_triggers(manager, User::Table.to_string()).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        trigger::drop_date_triggers(manager, User::Table.to_string()).await?;

        manager
            .drop_table(Table::drop().table(User::Table).to_owned())
            .await
    }
}

/// Learn more at https://docs.rs/sea-query#iden
#[derive(Iden)]
pub enum User {
    Table,
    Id,
    Permissions,
    Username,
    Email,
    PasswordHash,
}
