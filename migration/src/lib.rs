pub use sea_orm_migration::prelude::*;

mod m20220604_195200_create_manga_table;
mod m20220607_015227_create_chapter_table;
mod m20220613_113414_create_user_table;
mod m20220613_124113_create_reading_table;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20220604_195200_create_manga_table::Migration),
            Box::new(m20220607_015227_create_chapter_table::Migration),
            Box::new(m20220613_113414_create_user_table::Migration),
            Box::new(m20220613_124113_create_reading_table::Migration),
        ]
    }
}
