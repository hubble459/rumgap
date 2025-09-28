use sea_orm_migration::prelude::*;

use crate::m20221127_174334_create_user::User;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(Iden)]
enum UserWithDeviceIds {
    DeviceIds,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(User::Table)
                    .add_column_if_not_exists(
                        ColumnDef::new(UserWithDeviceIds::DeviceIds)
                            .array(ColumnType::String(Default::default()))
                            .default(Expr::cust(r#"'{}'"#))
                            .not_null(),
                    )
                    .take(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(User::Table)
                    .drop_column(UserWithDeviceIds::DeviceIds)
                    .take(),
            )
            .await
    }
}
