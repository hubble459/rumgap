use sea_orm_migration::prelude::*;

use crate::m20221127_174334_create_user::User;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let device_ids: Alias = Alias::new("device_ids");

        manager
            .alter_table(
                Table::alter()
                    .table(User::Table)
                    .add_column_if_not_exists(
                        ColumnDef::new(device_ids)
                            .array(ColumnType::String(None))
                            .default(Expr::cust(
                                r#"'{}'"#,
                            ))
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let device_ids: Alias = Alias::new("device_ids");

        manager
            .alter_table(
                Table::alter()
                    .table(User::Table)
                    .drop_column(device_ids)
                    .to_owned(),
            )
            .await
    }
}
