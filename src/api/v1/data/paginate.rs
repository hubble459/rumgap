use std::fmt::Debug;

use actix_web::{Responder, http::header::ContentType, body::BoxBody, HttpResponse};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct Paginate<T>
where
    T: Serialize + IntoIterator + Debug,
{
    pub total: u64,
    pub max_page: u64,
    pub page: u64,
    pub limit: u64,
    pub items: T,
}

impl<T: Serialize + IntoIterator + Debug> Responder for Paginate<T> {
    type Body = BoxBody;

    fn respond_to(self, _req: &actix_web::HttpRequest) -> actix_web::HttpResponse<Self::Body> {
        let body = serde_json::to_string(&self).unwrap();

        // Create response and set content type
        HttpResponse::Ok()
            .content_type(ContentType::json())
            .body(body)
    }
}

fn default_page() -> u64 {
    0
}

fn default_limit() -> u64 {
    20
}

#[derive(Debug, Deserialize)]
pub struct PaginateQuery {
    #[serde(default = "default_page")]
    pub page: u64,
    #[serde(default = "default_limit")]
    pub limit: u64,
}
