// use rocket::{http::Status, serde::json::Json, Route};
// use sea_orm_rocket::Connection;
// use serde_json::Value;

// use crate::{auth::User, pagination::Pagination, pool::Db};

// use super::manga::DEFAULT_LIMIT;

// #[get("/?<page>&<limit>")]
// async fn index(
//     conn: Connection<'_, Db>,
//     page: Option<usize>,
//     limit: Option<usize>,
//     user: User,
// ) -> Result<Json<Pagination<Vec<Value>>>, Status> {
//     let page = page.unwrap_or(1);
//     let limit = limit.unwrap_or(DEFAULT_LIMIT);
//     if page == 0 || limit == 0 {
//         return Err(Status::BadRequest);
//     }

//     let db = conn.into_inner();

//     let paginator = Reading::find()
//         .filter(Reading::Column::MangaId.eq(manga_id))
//         .order_by_asc(Reading::Column::Number)
//         .order_by_asc(Reading::Column::Posted)
//         .into_json()
//         .paginate(db, limit);
//     let num_pages = paginator.num_pages().await.ok().unwrap();

//     let reading = paginator
//         .fetch_page(page - 1)
//         .await
//         .map_err(|_| Status::InternalServerError)?;

//     Ok(Json(Pagination {
//         num_pages,
//         page,
//         limit,
//         data: reading,
//     }))
// }

// pub fn routes() -> Vec<Route> {
//     routes![index]
// }

// pub fn base() -> &'static str {
//     "reading"
// }
