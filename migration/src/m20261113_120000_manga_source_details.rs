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
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Every source keeps the details its own scrape found. `details_updated_at` is NULL
        // until a source has been scraped since this migration.
        execute(
            manager,
            r#"
            ALTER TABLE manga_source
                ADD COLUMN title varchar(511) NOT NULL DEFAULT '',
                ADD COLUMN description varchar NOT NULL DEFAULT '',
                ADD COLUMN status varchar(255) NOT NULL DEFAULT 'Ongoing',
                ADD COLUMN is_ongoing boolean NOT NULL DEFAULT true,
                ADD COLUMN genres varchar(255)[] NOT NULL DEFAULT '{}',
                ADD COLUMN authors varchar(255)[] NOT NULL DEFAULT '{}',
                ADD COLUMN alt_titles varchar(255)[] NOT NULL DEFAULT '{}',
                ADD COLUMN details_updated_at timestamp,
                ADD COLUMN cover_source_url varchar(511),
                ADD COLUMN cover_status varchar(255) NOT NULL DEFAULT 'pending',
                ADD COLUMN cover_storage_key varchar(511),
                ADD COLUMN cover_content_type varchar(255),
                ADD COLUMN cover_checksum varchar(64),
                ADD COLUMN cover_attempts integer NOT NULL DEFAULT 0
            "#,
        )
        .await?;

        // Until each source is scraped again, every source starts out with the details the
        // manga showed so far (its primary's). Only the primary gets the stored cover: the
        // file is keyed by manga id and must not end up shared between sources.
        execute(
            manager,
            r#"
            UPDATE manga_source ms SET
                title = m.title,
                description = m.description,
                status = m.status,
                is_ongoing = m.is_ongoing,
                genres = m.genres,
                authors = m.authors,
                alt_titles = m.alt_titles,
                details_updated_at = CASE WHEN ms.is_primary THEN m.updated_at END,
                cover_source_url = CASE WHEN ms.is_primary THEN m.cover_source_url END,
                cover_status = CASE WHEN ms.is_primary THEN m.cover_status ELSE 'pending' END,
                cover_storage_key = CASE WHEN ms.is_primary THEN m.cover_storage_key END,
                cover_content_type = CASE WHEN ms.is_primary THEN m.cover_content_type END,
                cover_checksum = CASE WHEN ms.is_primary THEN m.cover_checksum END,
                cover_attempts = CASE WHEN ms.is_primary THEN m.cover_attempts ELSE 0 END
            FROM manga m
            WHERE m.id = ms.manga_id
            "#,
        )
        .await?;

        // The source a user last picked for a manga in their reading list.
        execute(
            manager,
            r#"
            ALTER TABLE reading
                ADD COLUMN preferred_source_id integer
                REFERENCES manga_source (id) ON UPDATE CASCADE ON DELETE SET NULL
            "#,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        execute(manager, "ALTER TABLE reading DROP COLUMN preferred_source_id").await?;
        execute(
            manager,
            r#"
            ALTER TABLE manga_source
                DROP COLUMN title, DROP COLUMN description, DROP COLUMN status, DROP COLUMN is_ongoing,
                DROP COLUMN genres, DROP COLUMN authors, DROP COLUMN alt_titles, DROP COLUMN details_updated_at,
                DROP COLUMN cover_source_url, DROP COLUMN cover_status, DROP COLUMN cover_storage_key,
                DROP COLUMN cover_content_type, DROP COLUMN cover_checksum, DROP COLUMN cover_attempts
            "#,
        )
        .await
    }
}
