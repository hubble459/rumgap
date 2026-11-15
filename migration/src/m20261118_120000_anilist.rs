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
    // A user's AniList connection (tokens last a year and can't be refreshed), and each
    // manga's AniList match with a cache of its AniList details.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        execute(
            manager,
            "ALTER TABLE \"user\" ADD COLUMN anilist_token text, \
             ADD COLUMN anilist_token_expires_at timestamp, \
             ADD COLUMN anilist_user_id integer, \
             ADD COLUMN anilist_user_name varchar",
        )
        .await?;
        execute(
            manager,
            "ALTER TABLE manga ADD COLUMN anilist_id integer, \
             ADD COLUMN anilist_match varchar, \
             ADD COLUMN anilist_matched_at timestamp, \
             ADD COLUMN anilist_title varchar, \
             ADD COLUMN anilist_description text, \
             ADD COLUMN anilist_cover varchar, \
             ADD COLUMN anilist_banner varchar, \
             ADD COLUMN anilist_score integer, \
             ADD COLUMN anilist_genres text[], \
             ADD COLUMN anilist_status varchar, \
             ADD COLUMN anilist_fetched_at timestamp",
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        execute(
            manager,
            "ALTER TABLE manga DROP COLUMN anilist_id, DROP COLUMN anilist_match, \
             DROP COLUMN anilist_matched_at, DROP COLUMN anilist_title, \
             DROP COLUMN anilist_description, DROP COLUMN anilist_cover, \
             DROP COLUMN anilist_banner, DROP COLUMN anilist_score, \
             DROP COLUMN anilist_genres, DROP COLUMN anilist_status, \
             DROP COLUMN anilist_fetched_at",
        )
        .await?;
        execute(
            manager,
            "ALTER TABLE \"user\" DROP COLUMN anilist_token, DROP COLUMN anilist_token_expires_at, \
             DROP COLUMN anilist_user_id, DROP COLUMN anilist_user_name",
        )
        .await
    }
}
