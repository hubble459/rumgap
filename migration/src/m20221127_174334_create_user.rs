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
                    .table(User::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(User::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(User::Permissions)
                            .small_integer()
                            .not_null()
                            .default(1),
                    )
                    .col(
                        ColumnDef::new(User::Username)
                            .string_len(15)
                            .unique_key()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(User::Email)
                            .string_len(255)
                            .unique_key()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(User::PasswordHash)
                            .string_len(255)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(User::PreferredHostnames)
                            .array(ColumnType::String(None))
                            .default(Expr::cust(
                                r#"'{"mangadex.org","isekaiscan.com","manganato.com"}'"#,
                            ))
                            .not_null(),
                    )
                    .take(),
            )
            .await?;

        manager.timestamps(User::Table).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(User::Table).take())
            .await
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
