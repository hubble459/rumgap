use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ConnectionTrait, Statement};

use crate::m20221130_215753_create_reading::Reading;
use crate::m20260820_120003_create_canonical_chapter::CanonicalChapter;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Reading::Table)
                    .add_column_if_not_exists(ColumnDef::new(Reading::LastCanonicalChapterId).integer())
                    .take(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Reading::Table)
                    .add_foreign_key(
                        TableForeignKey::new()
                            .name("reading_last_canonical_chapter_id_fkey")
                            .from_tbl(Reading::Table)
                            .from_col(Reading::LastCanonicalChapterId)
                            .to_tbl(CanonicalChapter::Table)
                            .to_col(CanonicalChapter::Id)
                            .on_delete(ForeignKeyAction::SetNull)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .take(),
            )
            .await?;

        // Backfill: `reading.progress` is an existing int count of chapters read. Since the
        // previous migration's backfill assigned ordinals 1, 2, 3... in insertion order per
        // manga, `progress` (a count) already lines up 1:1 with the ordinal of the
        // last-read canonical chapter for the common (single-source, at-the-time) case.
        manager
            .get_connection()
            .execute_raw(Statement::from_string(
                manager.get_database_backend(),
                String::from(
                    r#"
                        UPDATE "reading"
                        SET "last_canonical_chapter_id" = "canonical_chapter"."id"
                        FROM "canonical_chapter"
                        WHERE "canonical_chapter"."manga_id" = "reading"."manga_id"
                          AND "canonical_chapter"."ordinal" = "reading"."progress"
                          AND "reading"."progress" > 0;
                    "#,
                ),
            ))
            .await
            .map(|_| ())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Reading::Table)
                    .drop_foreign_key("reading_last_canonical_chapter_id_fkey")
                    .take(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Reading::Table)
                    .drop_column(Reading::LastCanonicalChapterId)
                    .take(),
            )
            .await
    }
}
