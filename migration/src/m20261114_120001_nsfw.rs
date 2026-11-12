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
    // `manga.nsfw` marks 18+ manga: set for every source from an adult-only site, and by
    // mods/admins. `user.show_nsfw` is the user's choice to see them outside their own
    // reading list.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        execute(
            manager,
            "ALTER TABLE manga ADD COLUMN nsfw boolean NOT NULL DEFAULT false",
        )
        .await?;
        execute(
            manager,
            r#"ALTER TABLE "user" ADD COLUMN show_nsfw boolean NOT NULL DEFAULT true"#,
        )
        .await?;
        execute(
            manager,
            "UPDATE manga SET nsfw = true WHERE id IN (SELECT manga_id FROM manga_source WHERE hostname = 'hentai20.io')",
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        execute(manager, "ALTER TABLE manga DROP COLUMN nsfw").await?;
        execute(manager, r#"ALTER TABLE "user" DROP COLUMN show_nsfw"#).await
    }
}
