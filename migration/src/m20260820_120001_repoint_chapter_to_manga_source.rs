use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ConnectionTrait, Statement};

use crate::m20221130_215742_create_manga::Manga;
use crate::m20221130_215749_create_chapter::Chapter;
use crate::m20260820_120000_create_manga_source::MangaSource;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 1. Add the new column, nullable for now so it can be backfilled.
        manager
            .alter_table(
                Table::alter()
                    .table(Chapter::Table)
                    .add_column_if_not_exists(ColumnDef::new(Chapter::MangaSourceId).integer())
                    .take(),
            )
            .await?;

        // 2. Backfill: every manga has exactly one manga_source at this point (the one
        // created in the previous migration), so this is an unambiguous 1:1 mapping.
        manager
            .get_connection()
            .execute_raw(Statement::from_string(
                manager.get_database_backend(),
                String::from(
                    r#"
                        UPDATE "chapter"
                        SET "manga_source_id" = "manga_source"."id"
                        FROM "manga_source"
                        WHERE "chapter"."manga_id" = "manga_source"."manga_id";
                    "#,
                ),
            ))
            .await?;

        // 3. Now that every row is backfilled, enforce NOT NULL.
        manager
            .alter_table(
                Table::alter()
                    .table(Chapter::Table)
                    .modify_column(ColumnDef::new(Chapter::MangaSourceId).integer().not_null())
                    .take(),
            )
            .await?;

        // 4. Add the new FK.
        manager
            .alter_table(
                Table::alter()
                    .table(Chapter::Table)
                    .add_foreign_key(
                        TableForeignKey::new()
                            .name("chapter_manga_source_id_fkey")
                            .from_tbl(Chapter::Table)
                            .from_col(Chapter::MangaSourceId)
                            .to_tbl(MangaSource::Table)
                            .to_col(MangaSource::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .take(),
            )
            .await?;

        // 5. Drop the old FK. Confirmed via `psql \d chapter` against a throwaway test
        // database (SeaORM's default-generated name for this constraint).
        manager
            .alter_table(
                Table::alter()
                    .table(Chapter::Table)
                    .drop_foreign_key("chapter_manga_id_fkey")
                    .take(),
            )
            .await?;

        // 6. Drop the now-redundant column.
        manager
            .alter_table(
                Table::alter()
                    .table(Chapter::Table)
                    .drop_column(Chapter::MangaId)
                    .take(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 1. Re-add manga_id, nullable for backfill.
        manager
            .alter_table(
                Table::alter()
                    .table(Chapter::Table)
                    .add_column_if_not_exists(ColumnDef::new(Chapter::MangaId).integer())
                    .take(),
            )
            .await?;

        // 2. Backfill from manga_source.
        manager
            .get_connection()
            .execute_raw(Statement::from_string(
                manager.get_database_backend(),
                String::from(
                    r#"
                        UPDATE "chapter"
                        SET "manga_id" = "manga_source"."manga_id"
                        FROM "manga_source"
                        WHERE "chapter"."manga_source_id" = "manga_source"."id";
                    "#,
                ),
            ))
            .await?;

        // 3. Enforce NOT NULL again.
        manager
            .alter_table(
                Table::alter()
                    .table(Chapter::Table)
                    .modify_column(ColumnDef::new(Chapter::MangaId).integer().not_null())
                    .take(),
            )
            .await?;

        // 4. Restore the original FK.
        manager
            .alter_table(
                Table::alter()
                    .table(Chapter::Table)
                    .add_foreign_key(
                        TableForeignKey::new()
                            .name("chapter_manga_id_fkey")
                            .from_tbl(Chapter::Table)
                            .from_col(Chapter::MangaId)
                            .to_tbl(Manga::Table)
                            .to_col(Manga::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .take(),
            )
            .await?;

        // 5. Drop the manga_source_id FK and column.
        manager
            .alter_table(
                Table::alter()
                    .table(Chapter::Table)
                    .drop_foreign_key("chapter_manga_source_id_fkey")
                    .take(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Chapter::Table)
                    .drop_column(Chapter::MangaSourceId)
                    .take(),
            )
            .await
    }
}
