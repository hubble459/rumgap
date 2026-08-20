use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ConnectionTrait, Statement};

use crate::extension::timestamps::TimestampExt;
use crate::m20221130_215742_create_manga::Manga;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MangaSource::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(MangaSource::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(MangaSource::MangaId).integer().not_null())
                    .col(ColumnDef::new(MangaSource::Url).string_len(511).not_null().unique_key())
                    .col(ColumnDef::new(MangaSource::Hostname).string_len(255).not_null())
                    .col(
                        ColumnDef::new(MangaSource::Language)
                            .string_len(64)
                            .not_null()
                            .default("unknown"),
                    )
                    .col(
                        ColumnDef::new(MangaSource::IsPrimary)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(MangaSource::Table, MangaSource::MangaId)
                            .to(Manga::Table, Manga::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .take(),
            )
            .await?;

        manager.timestamps(MangaSource::Table).await?;

        // Backfill: every existing manga becomes the primary (and only) source of itself,
        // with `hostname` derived from its url.
        manager
            .get_connection()
            .execute_raw(Statement::from_string(
                manager.get_database_backend(),
                String::from(
                    r#"
                        INSERT INTO "manga_source" ("manga_id", "url", "hostname", "language", "is_primary", "created_at", "updated_at")
                        SELECT "id", "url", substring("url" from '://([^/]+)'), 'unknown', true, "created_at", "updated_at"
                        FROM "manga";
                    "#,
                ),
            ))
            .await
            .map(|_| ())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(MangaSource::Table).take()).await
    }
}

/// Learn more at https://docs.rs/sea-query#iden
#[derive(Iden)]
pub(crate) enum MangaSource {
    Table,
    Id,
    MangaId,
    Url,
    Hostname,
    Language,
    IsPrimary,
}
