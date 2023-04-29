use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ConnectionTrait, Statement};

#[derive(Iden)]
pub enum Timestamp {
    CreatedAt,
    UpdatedAt,
}

#[async_trait::async_trait]
pub trait TimestampExt {
    async fn timestamps<T>(&self, table: T) -> Result<(), DbErr>
    where
        T: Iden;
    async fn drop_timestamps<T>(&self, table: T) -> Result<(), DbErr>
    where
        T: Iden;
}

#[async_trait::async_trait]
impl<'a> TimestampExt for SchemaManager<'a> {
    async fn timestamps<T>(&self, table: T) -> Result<(), DbErr>
    where
        T: Iden,
    {
        let table_name = table.to_string();

        self.alter_table(
            Table::alter()
                .table(Alias::new(&table_name))
                .add_column_if_not_exists(
                    ColumnDef::new(Timestamp::CreatedAt)
                        .timestamp()
                        .extra("DEFAULT NOW()".to_owned())
                        .not_null(),
                )
                .add_column_if_not_exists(
                    ColumnDef::new(Timestamp::UpdatedAt)
                        .timestamp()
                        .extra("DEFAULT NOW()".to_owned())
                        .not_null(),
                )
                .take(),
        )
        .await?;

        self.get_connection()
            .execute(Statement::from_string(
                self.get_database_backend(),
                format!(
                    r#"
                        CREATE TRIGGER set_{table_name}_timestamp
                        BEFORE UPDATE ON "{table_name}"
                        FOR EACH ROW
                        EXECUTE PROCEDURE trigger_set_timestamp();
                    "#,
                ),
            ))
            .await
            .map(|_| ())
    }

    async fn drop_timestamps<T>(&self, table: T) -> Result<(), DbErr>
    where
        T: Iden,
    {
        let table_name = table.to_string();

        self.alter_table(
            Table::alter()
                .table(Alias::new(&table_name))
                .drop_column(Timestamp::CreatedAt)
                .drop_column(Timestamp::UpdatedAt)
                .take(),
        )
        .await?;

        self.get_connection()
            .execute(Statement::from_string(
                self.get_database_backend(),
                format!(
                    r#"DROP TRIGGER IF EXISTS set_{}_timestamp ON "{}""#,
                    table_name, table_name,
                ),
            ))
            .await
            .map(|_| ())
    }
}
