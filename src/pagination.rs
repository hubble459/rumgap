use rocket::serde::Serialize;

#[derive(Serialize)]
#[serde(crate = "rocket::serde")]
pub struct Pagination<T: Serialize> {
    pub page: u64,
    pub limit: u64,
    pub num_items: u64,
    pub num_pages: u64,
    pub items: T,
}
