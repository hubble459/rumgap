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
mod m20260820_120007_add_cover_tracking_to_manga;
mod m20260820_120008_add_dimensions_to_chapter_image;
mod m20260826_120000_create_scrape_log;
mod m20261112_120000_index_chapter_manga_source_id;
mod m20261112_120001_create_session;
mod m20261113_120000_manga_source_details;
mod m20261113_120001_drop_manga_details;
mod m20261113_120002_unbounded_source_details;
mod m20261114_120000_create_manga_dismissal;
mod m20261114_120001_nsfw;
mod m20261115_120000_social;
mod m20261116_120000_mangadex_hostname;

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
            Box::new(m20260820_120007_add_cover_tracking_to_manga::Migration),
            Box::new(m20260820_120008_add_dimensions_to_chapter_image::Migration),
            Box::new(m20260826_120000_create_scrape_log::Migration),
            Box::new(m20261112_120000_index_chapter_manga_source_id::Migration),
            Box::new(m20261112_120001_create_session::Migration),
            Box::new(m20261113_120000_manga_source_details::Migration),
            Box::new(m20261113_120001_drop_manga_details::Migration),
            Box::new(m20261113_120002_unbounded_source_details::Migration),
            Box::new(m20261114_120000_create_manga_dismissal::Migration),
            Box::new(m20261114_120001_nsfw::Migration),
            Box::new(m20261115_120000_social::Migration),
            Box::new(m20261116_120000_mangadex_hostname::Migration),
        ]
    }
}
