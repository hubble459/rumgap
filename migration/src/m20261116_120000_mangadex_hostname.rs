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
    // manga_parser now stores every mangadex manga as https://mangadex.org/title/{id}, where
    // search used to give https://api.mangadex.org/manga/{id}. Move existing sources (and
    // the hostnames that refer to them) over, skipping any whose new url is already taken.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        execute(
            manager,
            r#"
            UPDATE manga_source s
            SET url = 'https://mangadex.org/title/' || substring(s.url from '([0-9a-f-]{36})'),
                hostname = 'mangadex.org'
            WHERE s.hostname IN ('api.mangadex.org', 'mangadex.org')
              AND s.url ~ '/(manga|title)/[0-9a-f-]{36}'
              AND s.url <> 'https://mangadex.org/title/' || substring(s.url from '([0-9a-f-]{36})')
              AND NOT EXISTS (
                  SELECT 1 FROM manga_source other
                  WHERE other.url = 'https://mangadex.org/title/' || substring(s.url from '([0-9a-f-]{36})')
              )
            "#,
        )
        .await?;
        execute(
            manager,
            r#"UPDATE "user" SET preferred_hostnames = array_remove(preferred_hostnames, 'api.mangadex.org')
               WHERE 'mangadex.org' = ANY(preferred_hostnames)"#,
        )
        .await?;
        execute(
            manager,
            r#"UPDATE "user" SET preferred_hostnames = array_replace(preferred_hostnames, 'api.mangadex.org', 'mangadex.org')
               WHERE 'api.mangadex.org' = ANY(preferred_hostnames)"#,
        )
        .await?;
        execute(
            manager,
            "UPDATE scrape_log SET hostname = 'mangadex.org' WHERE hostname = 'api.mangadex.org'",
        )
        .await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Which sources had an api.mangadex.org url isn't kept; both forms keep working.
        Ok(())
    }
}
