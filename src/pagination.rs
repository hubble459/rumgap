use rocket::serde::Serialize;

#[derive(Serialize)]
#[serde(crate = "rocket::serde")]
pub struct Pagination<T> where T: Serialize {
    pub page: usize,
    pub limit: usize,
    pub num_pages: usize,
    pub data: T,
}
