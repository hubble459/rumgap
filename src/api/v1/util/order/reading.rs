use actix_web::Result;
use migration::{Order, SimpleExpr};

static ORDER_FIELD: phf::Map<&'static str, &'static str> = phf_map! {
    "progress" => "reading.title",
    "title" => "manga.title",
    "chapters" => "COUNT(chapter.id)",
    "updated" => "reading.updated_at",
    "created" => "reading.created_at",
};

pub fn parse(order: &str) -> Result<Vec<(SimpleExpr, Order)>> {
    super::parse(&ORDER_FIELD, order)
}
