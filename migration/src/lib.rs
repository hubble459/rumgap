pub use sea_orm_migration::prelude::*;

mod extension;

mod m20221127_174330_create_triggers;
mod m20221127_174334_create_user;
mod m20221127_180216_create_friend;
mod m20221130_215742_create_manga;
mod m20221130_215749_create_chapter;
mod m20221130_215753_create_reading;
mod m20230206_144400_create_chapter_offset;
mod m20230212_132547_add_page_column_to_chapter_offset;
mod m20230219_170615_add_device_ids_column_to_user;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20221127_174330_create_triggers::Migration),
            Box::new(m20221127_174334_create_user::Migration),
            Box::new(m20221127_180216_create_friend::Migration),
            Box::new(m20221130_215742_create_manga::Migration),
            Box::new(m20221130_215749_create_chapter::Migration),
            Box::new(m20221130_215753_create_reading::Migration),
            Box::new(m20230206_144400_create_chapter_offset::Migration),
            Box::new(m20230212_132547_add_page_column_to_chapter_offset::Migration),
            Box::new(m20230219_170615_add_device_ids_column_to_user::Migration),
        ]
    }
}
