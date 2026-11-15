use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::Statement;

#[derive(DeriveMigrationName)]
pub struct Migration;

async fn execute(manager: &SchemaManager<'_>, sql: &str) -> Result<(), DbErr> {
    manager
        .get_connection()
        .execute_raw(Statement::from_string(manager.get_database_backend(), sql.to_owned()))
        .await
        .map(|_| ())
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    // When the chapter disappeared from its source site. Such a chapter is only kept because
    // every page of it is stored here.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        execute(manager, "ALTER TABLE chapter ADD COLUMN removed_at timestamp").await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        execute(manager, "ALTER TABLE chapter DROP COLUMN removed_at").await
    }
}
