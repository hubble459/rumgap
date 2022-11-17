use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20220604_195200_create_manga_table"
    }
}

#[derive(Iden)]
enum Manga {
    Table,
    Id,
    Url,
    Title,
    Description,
    Cover,
    Ongoing,
    Authors,
    Genres,
    AltTitles,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Manga::Table)
                    .col(
                        ColumnDef::new(Manga::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Manga::Url)
                            .string_len(2048)
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(Manga::Title).text().not_null())
                    .col(ColumnDef::new(Manga::Description).text().not_null())
                    .col(ColumnDef::new(Manga::Cover).text())
                    .col(ColumnDef::new(Manga::Ongoing).boolean().default(true))
                    .col(ColumnDef::new(Manga::Genres).text())
                    .col(ColumnDef::new(Manga::AltTitles).text())
                    .col(ColumnDef::new(Manga::Authors).text())
                    .take(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Manga::Table).take())
            .await
    }
}
