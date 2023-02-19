use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::FromQueryResult;

use crate::proto::UserFullReply;

#[derive(Debug, FromQueryResult)]
pub struct Full {
    pub id: i32,
    pub permissions: i16,
    pub username: String,
    pub email: String,
    pub preferred_hostnames: Vec<String>,
    pub device_ids: Vec<String>,
    pub count_following: i64,
    pub count_followers: i64,
    // pub count_reading: i64,
    // pub count_planned: i64,
    // pub count_completed: i64,
    // pub count_dropped: i64,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

impl From<Full> for UserFullReply {
    fn from(value: Full) -> Self {
        Self {
            id: value.id,
            username: value.username,
            email: value.email,
            permissions: value.permissions as i32,
            preferred_hostnames: value.preferred_hostnames,
            device_ids: value.device_ids,
            count_followers: value.count_followers,
            count_following: value.count_following,
            created_at: value.created_at.timestamp_millis(),
            updated_at: value.updated_at.timestamp_millis(),
        }
    }
}
