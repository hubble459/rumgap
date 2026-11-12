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
    // Details live on the sources since m20261113_120000; `manga` only groups them.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        execute(
            manager,
            r#"
            ALTER TABLE manga
                DROP COLUMN title, DROP COLUMN description, DROP COLUMN status, DROP COLUMN is_ongoing,
                DROP COLUMN genres, DROP COLUMN authors, DROP COLUMN alt_titles,
                DROP COLUMN cover_source_url, DROP COLUMN cover_status, DROP COLUMN cover_storage_key,
                DROP COLUMN cover_content_type, DROP COLUMN cover_checksum, DROP COLUMN cover_attempts
            "#,
        )
        .await
    }

    // Restores the columns, filled from each manga's primary source.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        execute(
            manager,
            r#"
            ALTER TABLE manga
                ADD COLUMN title varchar(511) NOT NULL DEFAULT '',
                ADD COLUMN description varchar NOT NULL DEFAULT '',
                ADD COLUMN status varchar(255) NOT NULL DEFAULT 'Ongoing',
                ADD COLUMN is_ongoing boolean NOT NULL DEFAULT true,
                ADD COLUMN genres varchar(255)[] NOT NULL DEFAULT '{}',
                ADD COLUMN authors varchar(255)[] NOT NULL DEFAULT '{}',
                ADD COLUMN alt_titles varchar(255)[] NOT NULL DEFAULT '{}',
                ADD COLUMN cover_source_url varchar(511),
                ADD COLUMN cover_status varchar(255) NOT NULL DEFAULT 'pending',
                ADD COLUMN cover_storage_key varchar(511),
                ADD COLUMN cover_content_type varchar(255),
                ADD COLUMN cover_checksum varchar(64),
                ADD COLUMN cover_attempts integer NOT NULL DEFAULT 0
            "#,
        )
        .await?;
        execute(
            manager,
            r#"
            UPDATE manga m SET
                title = ms.title, description = ms.description, status = ms.status,
                is_ongoing = ms.is_ongoing, genres = ms.genres, authors = ms.authors,
                alt_titles = ms.alt_titles, cover_source_url = ms.cover_source_url,
                cover_status = ms.cover_status, cover_storage_key = ms.cover_storage_key,
                cover_content_type = ms.cover_content_type, cover_checksum = ms.cover_checksum,
                cover_attempts = ms.cover_attempts
            FROM manga_source ms
            WHERE ms.manga_id = m.id AND ms.is_primary
            "#,
        )
        .await
    }
}
