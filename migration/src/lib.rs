pub use sea_orm_migration::prelude::*;

#[allow(hidden_glob_reexports)]
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
mod m20231116_195236_fix_timestamps;
mod m20231125_223257_add_status_to_manga;
mod m20260820_120000_create_manga_source;
mod m20260820_120001_repoint_chapter_to_manga_source;
mod m20260820_120002_drop_manga_url;
mod m20260820_120003_create_canonical_chapter;
mod m20260820_120004_reading_canonical_progress;
mod m20260820_120005_chapter_offset_fraction;
mod m20260820_120006_create_chapter_image;

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
            Box::new(m20231116_195236_fix_timestamps::Migration),
            Box::new(m20231125_223257_add_status_to_manga::Migration),
            Box::new(m20260820_120000_create_manga_source::Migration),
            Box::new(m20260820_120001_repoint_chapter_to_manga_source::Migration),
            Box::new(m20260820_120002_drop_manga_url::Migration),
            Box::new(m20260820_120003_create_canonical_chapter::Migration),
            Box::new(m20260820_120004_reading_canonical_progress::Migration),
            Box::new(m20260820_120005_chapter_offset_fraction::Migration),
            Box::new(m20260820_120006_create_chapter_image::Migration),
        ]
    }
}
