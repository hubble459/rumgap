use sea_orm::Statement;
use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::ConnectionTrait;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20220626_215645_manga_fulltext"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_database_backend();
        manager.get_connection().execute(Statement::from_sql_and_values(
                db,
                "ALTER TABLE manga ADD FULLTEXT(title, description, genres, authors, alt_titles)",
                vec![],
            ))
            .await
            .map(|_| ())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_database_backend();
        manager.get_connection().execute(Statement::from_sql_and_values(
                db,
                "DROP FULLTEXT INDEX ON manga",
                vec![],
            ))
            .await
            .map(|_| ())
    }
}
