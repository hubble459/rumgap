use sea_orm_migration::prelude::*;

use crate::extension::timestamps::Timestamp;
use crate::m20221127_174334_create_user::User;
use crate::m20221127_180216_create_friend::Friend;
use crate::m20221130_215742_create_manga::Manga;
use crate::m20221130_215749_create_chapter::Chapter;
use crate::m20221130_215753_create_reading::Reading;
use crate::m20230206_144400_create_chapter_offset::ChapterOffset;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let tables = [
            User::Table.into_table_ref(),
            Friend::Table.into_table_ref(),
            Manga::Table.into_table_ref(),
            Chapter::Table.into_table_ref(),
            Reading::Table.into_table_ref(),
            ChapterOffset::Table.into_table_ref(),
        ];
        for table in tables {
            manager
                .alter_table(
                    Table::alter()
                        .table(table)
                        .modify_column(ColumnDef::new(Timestamp::CreatedAt).timestamp())
                        .modify_column(ColumnDef::new(Timestamp::UpdatedAt).timestamp())
                        .take(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
