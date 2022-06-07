use sea_orm_migration::prelude::*;
use entity::chapter::*;
use crate::sea_orm::Schema;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20220607_015227_create_chapter_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let builder = manager.get_database_backend();
        let schema = Schema::new(builder);
        manager
            .create_table(schema.create_table_from_entity(Entity))
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Entity).to_owned())
            .await
    }
}
