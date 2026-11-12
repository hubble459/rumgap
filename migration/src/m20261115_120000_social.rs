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
        // Private profiles show only their username to other users.
        execute(
            manager,
            r#"ALTER TABLE "user" ADD COLUMN is_private boolean NOT NULL DEFAULT true"#,
        )
        .await?;

        // Append-only reading history: `open` when a chapter is opened (again after a break),
        // `progress` whenever reading progress is set. `progress` is the reading progress after
        // a `progress` event.
        execute(
            manager,
            r#"
            CREATE TABLE read_event (
                id bigserial PRIMARY KEY,
                user_id integer NOT NULL REFERENCES "user"(id) ON UPDATE CASCADE ON DELETE CASCADE,
                manga_id integer NOT NULL REFERENCES manga(id) ON UPDATE CASCADE ON DELETE CASCADE,
                chapter_id integer REFERENCES chapter(id) ON UPDATE CASCADE ON DELETE SET NULL,
                kind varchar(16) NOT NULL,
                progress integer,
                created_at timestamp NOT NULL DEFAULT NOW()
            )
            "#,
        )
        .await?;
        execute(
            manager,
            "CREATE INDEX read_event_user_created ON read_event (user_id, created_at)",
        )
        .await?;
        execute(
            manager,
            "CREATE INDEX read_event_user_manga ON read_event (user_id, manga_id)",
        )
        .await?;
        execute(
            manager,
            r#"
            INSERT INTO read_event (user_id, manga_id, chapter_id, kind, created_at)
            SELECT co.user_id, s.manga_id, co.chapter_id, 'open', co.created_at
            FROM chapter_offset co
            JOIN chapter c ON c.id = co.chapter_id
            JOIN manga_source s ON s.id = c.manga_source_id
            ORDER BY co.created_at
            "#,
        )
        .await?;

        // Comments on a manga, threaded through `parent_id` to any depth. A deleted comment
        // that still has replies keeps its row (with `deleted_at` set and the body cleared) so
        // the thread under it survives.
        execute(
            manager,
            r#"
            CREATE TABLE comment (
                id serial PRIMARY KEY,
                manga_id integer NOT NULL REFERENCES manga(id) ON UPDATE CASCADE ON DELETE CASCADE,
                user_id integer REFERENCES "user"(id) ON UPDATE CASCADE ON DELETE SET NULL,
                parent_id integer REFERENCES comment(id) ON UPDATE CASCADE ON DELETE CASCADE,
                body text NOT NULL,
                edited_at timestamp,
                deleted_at timestamp,
                created_at timestamp NOT NULL DEFAULT NOW(),
                updated_at timestamp NOT NULL DEFAULT NOW()
            )
            "#,
        )
        .await?;
        execute(
            manager,
            "CREATE INDEX comment_manga_parent ON comment (manga_id, parent_id, created_at)",
        )
        .await?;
        execute(
            manager,
            "CREATE INDEX comment_parent ON comment (parent_id, created_at)",
        )
        .await?;
        execute(
            manager,
            r#"
            CREATE TRIGGER set_comment_timestamp
            BEFORE UPDATE ON comment
            FOR EACH ROW
            EXECUTE PROCEDURE trigger_set_timestamp()
            "#,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        execute(manager, "DROP TABLE comment").await?;
        execute(manager, "DROP TABLE read_event").await?;
        execute(manager, r#"ALTER TABLE "user" DROP COLUMN is_private"#).await
    }
}
