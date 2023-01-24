use migration::{Order, SimpleExpr};
use tonic::Status;

use crate::service::manga::NEXT_UPDATE_QUERY;

static ORDER_FIELD: phf::Map<&'static str, &'static str> = phf_map! {
    "title" => "manga.title",
    "description" => "manga.description",
    "url" => "manga.url",
    "last" => "MAX(chapter.posted)",
    "next" => NEXT_UPDATE_QUERY,
    "chapters" => "COUNT(chapter.id)",
};

pub fn parse(order: &str) -> Result<Vec<(SimpleExpr, Order)>, Status> {
    super::parse(&ORDER_FIELD, order)
}
