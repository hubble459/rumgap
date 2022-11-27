pub use sea_orm_migration::prelude::*;

mod m20221127_174330_create_triggers;
mod m20221127_174334_create_user;
mod m20221127_180216_create_friend;
mod trigger;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20221127_174330_create_triggers::Migration),
            Box::new(m20221127_174334_create_user::Migration),
            Box::new(m20221127_180216_create_friend::Migration),
        ]
    }
}
