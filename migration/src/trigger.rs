use sea_orm_migration::{
    prelude::*,
    sea_orm::{ConnectionTrait, Statement},
};

pub async fn drop_date_triggers<'a>(
    manager: &SchemaManager<'a>,
    table_name: String,
) -> Result<(), DbErr> {
    manager
        .get_connection()
        .execute(Statement::from_string(
            manager.get_database_backend(),
            format!(
                r#"DROP TRIGGER IF EXISTS set_{}_timestamp ON "{}""#,
                table_name, table_name,
            ),
        ))
        .await?;

    Ok(())
}

pub async fn add_date_triggers<'a>(
    manager: &SchemaManager<'a>,
    table_name: String,
) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Alias::new(&table_name))
                .add_column_if_not_exists(
                    ColumnDef::new(Timestamp::CreatedAt)
                        .timestamp_with_time_zone()
                        .extra("DEFAULT NOW()".to_owned())
                        .not_null(),
                )
                .add_column_if_not_exists(
                    ColumnDef::new(Timestamp::UpdatedAt)
                        .timestamp_with_time_zone()
                        .extra("DEFAULT NOW()".to_owned())
                        .not_null(),
                )
                .take(),
        )
        .await?;

    manager
        .get_connection()
        .execute(Statement::from_string(
            manager.get_database_backend(),
            format!(
                r#"
                    CREATE TRIGGER set_{}_timestamp
                    BEFORE UPDATE ON "{}"
                    FOR EACH ROW
                    EXECUTE PROCEDURE trigger_set_timestamp();
                "#,
                table_name, table_name,
            ),
        ))
        .await?;

    Ok(())
}

#[derive(Iden)]
enum Timestamp {
    CreatedAt,
    UpdatedAt,
}
