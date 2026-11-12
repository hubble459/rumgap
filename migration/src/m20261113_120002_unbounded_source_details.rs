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
    // Scraped text has no reliable maximum length (a site listing every alt title as one
    // string made a refresh fail on varchar(255)), so stop limiting it.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        execute(
            manager,
            r#"
            ALTER TABLE manga_source
                ALTER COLUMN title TYPE varchar,
                ALTER COLUMN status TYPE varchar,
                ALTER COLUMN genres TYPE varchar[],
                ALTER COLUMN authors TYPE varchar[],
                ALTER COLUMN alt_titles TYPE varchar[]
            "#,
        )
        .await
    }

    // An explicit cast to varchar(n) truncates whatever no longer fits.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        execute(
            manager,
            r#"
            ALTER TABLE manga_source
                ALTER COLUMN title TYPE varchar(511) USING title::varchar(511),
                ALTER COLUMN status TYPE varchar(255) USING status::varchar(255),
                ALTER COLUMN genres TYPE varchar(255)[] USING genres::varchar(255)[],
                ALTER COLUMN authors TYPE varchar(255)[] USING authors::varchar(255)[],
                ALTER COLUMN alt_titles TYPE varchar(255)[] USING alt_titles::varchar(255)[]
            "#,
        )
        .await
    }
}
