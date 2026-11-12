use sea_orm_migration::prelude::*;

use crate::m20221127_174334_create_user::User;
use crate::m20221130_215742_create_manga::Manga;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // A "not interested" swipe. It hides the manga from the user's "for you" deck until
        // the cooldown runs out, and counts against similar manga until then.
        manager
            .create_table(
                Table::create()
                    .table(MangaDismissal::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(MangaDismissal::UserId).integer().not_null())
                    .col(ColumnDef::new(MangaDismissal::MangaId).integer().not_null())
                    .col(
                        ColumnDef::new(MangaDismissal::CreatedAt)
                            .timestamp()
                            .extra("DEFAULT NOW()".to_owned())
                            .not_null(),
                    )
                    .primary_key(Index::create().col(MangaDismissal::UserId).col(MangaDismissal::MangaId))
                    .foreign_key(
                        ForeignKey::create()
                            .from(MangaDismissal::Table, MangaDismissal::UserId)
                            .to(User::Table, User::Id)
                            .on_update(ForeignKeyAction::Cascade)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(MangaDismissal::Table, MangaDismissal::MangaId)
                            .to(Manga::Table, Manga::Id)
                            .on_update(ForeignKeyAction::Cascade)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .take(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MangaDismissal::Table).take())
            .await
    }
}

#[derive(Iden)]
pub(crate) enum MangaDismissal {
    Table,
    UserId,
    MangaId,
    CreatedAt,
}
