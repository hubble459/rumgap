use rocket::serde::{Serialize, Deserialize};

#[derive(Serialize)]
#[serde(crate = "rocket::serde")]
pub struct Pagination<T: Serialize + for<'de> Deserialize<'de>> {
    pub page: u64,
    pub limit: u64,
    pub num_items: u64,
    pub num_pages: u64,
    pub items: T,
}
