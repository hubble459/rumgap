use sea_orm_migration::{
    prelude::*,
    schema::{array, pk_auto, small_integer, string, string_len_uniq, string_uniq},
};

use crate::extension::timestamps::TimestampExt;

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
                    .col(pk_auto(User::Id))
                    .col(small_integer(User::Permissions).default(1))
                    .col(string_len_uniq(User::Username, 15))
                    .col(string_uniq(User::Email))
                    .col(string(User::PasswordHash))
                    .col(array(User::PreferredHostnames, ColumnType::String(Default::default())))
                    .take(),
            )
            .await?;

        manager.timestamps(User::Table).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(User::Table).take()).await
    }
}

/// Learn more at https://docs.rs/sea-query#iden
#[derive(Iden)]
pub(crate) enum User {
    Table,
    Id,
    Permissions,
    Username,
    Email,
    PasswordHash,
    PreferredHostnames,
}
