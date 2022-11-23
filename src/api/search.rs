use manga_parser::{
    model::SearchManga,
    parser::{MangaParser, Parser},
};
use rocket::{http::Status, response::content::RawJson, serde::json::Json, Route, State};
use sea_orm::{query::*};
use serde_json::json;

#[get("/search?<keyword>&<hostnames>")]
async fn index(
    keyword: String,
    hostnames: Option<Vec<String>>,
    parser: &State<MangaParser>,
) -> Result<Json<Vec<SearchManga>>, (Status, RawJson<JsonValue>)> {
    let hostnames = hostnames.unwrap_or(vec![]);
    let results = parser.search(keyword, hostnames).await.map_err(|e| {
        (
            Status::InternalServerError,
            RawJson(json!({ "message": e.to_string() })),
        )
    })?;

    Ok(Json(results))
}

pub fn routes() -> Vec<Route> {
    routes![index]
}
