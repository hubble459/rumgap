use migration::{Expr, Order, SimpleExpr};
use regex::Regex;
use tonic::Status;

pub mod manga;

lazy_static! {
    static ref ORDER_REGEX: Regex = Regex::new(r"(\w+)(:(ASC|DESC|asc|desc))?").unwrap();
}

/// Order parser for generating a SeaORM query
pub fn parse(map: &phf::Map<&'static str, &'static str>, order: &str) -> Result<Vec<(SimpleExpr, Order)>, Status> {
    let capture_list = ORDER_REGEX.captures_iter(order);

    let mut orders = vec![];
    for captures in capture_list {
        let name = captures.get(1).unwrap().as_str();
        let column = map.get(name);
        if let Some(column) = column {
            let order = if let Some(order) = captures.get(3) {
                let order = order.as_str().to_ascii_lowercase();
                match order.as_str() {
                    "desc" => Order::Desc,
                    "asc" => Order::Asc,
                    _ => unreachable!(),
                }
            } else {
                Order::Asc
            };
            orders.push((Expr::cust(*column), order));
        } else {
            return Err(Status::invalid_argument(format!("Can not sort on {name}")));
        }
    }

    Ok(orders)
}
