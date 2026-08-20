use sea_orm_migration::prelude::*;

use crate::m20221130_215742_create_manga::Manga;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // `manga.url` is now fully redundant with `manga_source.url` (the primary source's
        // url in particular), which was backfilled from it two migrations ago.
        manager
            .alter_table(Table::alter().table(Manga::Table).drop_column(Manga::Url).take())
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Best-effort reversal only: the column is restored empty (nullable, no unique
        // constraint) since the original values now live on manga_source and a down
        // migration isn't expected to reconstruct historical uniqueness guarantees.
        manager
            .alter_table(
                Table::alter()
                    .table(Manga::Table)
                    .add_column_if_not_exists(ColumnDef::new(Manga::Url).string_len(511))
                    .take(),
            )
            .await
    }
}
