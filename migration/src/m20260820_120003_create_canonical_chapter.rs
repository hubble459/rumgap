use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ConnectionTrait, Statement};

use crate::extension::timestamps::TimestampExt;
use crate::m20221130_215742_create_manga::Manga;
use crate::m20221130_215749_create_chapter::Chapter;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(CanonicalChapter::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(CanonicalChapter::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(CanonicalChapter::MangaId).integer().not_null())
                    // NUMERIC, not float: ordinal equality is load-bearing for the matching
                    // heuristic and float equality is a classic footgun.
                    .col(ColumnDef::new(CanonicalChapter::Ordinal).decimal_len(10, 3).not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .from(CanonicalChapter::Table, CanonicalChapter::MangaId)
                            .to(Manga::Table, Manga::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .index(
                        Index::create()
                            .name("idx_canonical_chapter_manga_id_ordinal")
                            .table(CanonicalChapter::Table)
                            .col(CanonicalChapter::MangaId)
                            .col(CanonicalChapter::Ordinal)
                            .unique(),
                    )
                    .take(),
            )
            .await?;

        manager.timestamps(CanonicalChapter::Table).await?;

        // Add the (nullable) link from chapter -> canonical_chapter. NULL is reserved for the
        // manual UnlinkChapter override case; every other chapter always ends up linked, even
        // if only to a solo canonical row nobody else shares (yet).
        manager
            .alter_table(
                Table::alter()
                    .table(Chapter::Table)
                    .add_column_if_not_exists(ColumnDef::new(Chapter::CanonicalChapterId).integer())
                    .take(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Chapter::Table)
                    .add_foreign_key(
                        TableForeignKey::new()
                            .name("chapter_canonical_chapter_id_fkey")
                            .from_tbl(Chapter::Table)
                            .from_col(Chapter::CanonicalChapterId)
                            .to_tbl(CanonicalChapter::Table)
                            .to_col(CanonicalChapter::Id)
                            .on_delete(ForeignKeyAction::SetNull)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .take(),
            )
            .await?;

        // Backfill: every chapter is single-source today, so assign ordinals by insertion
        // order per manga (matches the actual current sort order, not the unreliable `number`
        // float) and link each chapter 1:1 to its own new canonical row.
        manager
            .get_connection()
            .execute_raw(Statement::from_string(
                manager.get_database_backend(),
                String::from(
                    r#"
                        WITH ranked AS (
                            SELECT "chapter"."id" AS chapter_id,
                                   "manga_source"."manga_id" AS manga_id,
                                   ROW_NUMBER() OVER (PARTITION BY "manga_source"."manga_id" ORDER BY "chapter"."id") AS ordinal
                            FROM "chapter"
                            JOIN "manga_source" ON "manga_source"."id" = "chapter"."manga_source_id"
                        ), inserted AS (
                            INSERT INTO "canonical_chapter" ("manga_id", "ordinal", "created_at", "updated_at")
                            SELECT "manga_id", "ordinal", NOW(), NOW() FROM ranked
                            RETURNING "id", "manga_id", "ordinal"
                        )
                        UPDATE "chapter"
                        SET "canonical_chapter_id" = "inserted"."id"
                        FROM "ranked", "inserted"
                        WHERE "chapter"."id" = "ranked"."chapter_id"
                          AND "inserted"."manga_id" = "ranked"."manga_id"
                          AND "inserted"."ordinal" = "ranked"."ordinal";
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
                    .table(Chapter::Table)
                    .drop_foreign_key("chapter_canonical_chapter_id_fkey")
                    .take(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Chapter::Table)
                    .drop_column(Chapter::CanonicalChapterId)
                    .take(),
            )
            .await?;

        manager
            .drop_table(Table::drop().table(CanonicalChapter::Table).take())
            .await
    }
}

/// Learn more at https://docs.rs/sea-query#iden
#[derive(Iden)]
pub(crate) enum CanonicalChapter {
    Table,
    Id,
    MangaId,
    Ordinal,
}
