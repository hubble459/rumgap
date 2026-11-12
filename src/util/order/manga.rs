use migration::{Order, SimpleExpr};
use tonic::Status;

use crate::service::v1::manga::NEXT_UPDATE_QUERY;

static ORDER_FIELD: phf::Map<&'static str, &'static str> = phf_map! {
    // These resolve through the source `index_manga` shows each manga through.
    "title" => "manga_source.title",
    "description" => "manga_source.description",
    "url" => "manga_source.url",
    "last" => "MAX(chapter.posted)",
    "next" => NEXT_UPDATE_QUERY,
    "chapters" => "COUNT(chapter.id)",
    // Can break when no reading.progress? dunno
    "reading" => "reading.progress",
    "progress" => "reading.progress",
    "last_read" => "reading.updated_at",
};

pub fn parse(order: &str) -> Result<Vec<(SimpleExpr, Order)>, Status> {
    super::parse(&ORDER_FIELD, order)
}
