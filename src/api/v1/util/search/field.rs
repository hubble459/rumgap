use actix_web::error::ErrorBadRequest;
use migration::{Expr, SimpleExpr};
use regex::Regex;

lazy_static! {
    static ref SEARCH_DATE_REGEX: Regex = Regex::new(r"([<>]=?)?(.+)").unwrap();
}

#[derive(Debug, Clone)]
pub enum SearchField {
    Array(&'static str),
    Text(&'static str),
    Date(&'static str),
    Equals(&'static str),
    Number(&'static str),
}

impl SearchField {
    pub fn into_expression(self, mut value: &str, exclude: bool) -> actix_web::Result<SimpleExpr> {
        let mut expr = String::new();

        if exclude {
            expr += "NOT ";
        }

        match self {
            SearchField::Array(ident) => {
                expr += &format!("ARRAY_TO_STRING({}, ', ') ILIKE $1", ident);
            }
            SearchField::Text(ident) => {
                expr += &format!("{} ILIKE $1", ident);
            }
            SearchField::Date(ident) => {
                if value.starts_with(">") {
                    value = &value[1..];
                } else if value.starts_with("<") {
                    expr += &format!("{} ILIKE $1", ident);
                    value = &value[1..];
                }
            }
            SearchField::Equals(ident) => {
                expr += &format!("{} = $1", ident);
            }
            SearchField::Number(ident) => {
                let captures = SEARCH_DATE_REGEX.captures(value).unwrap();
                let compare;

                if let Some(comp_match) = captures.get(1) {
                    compare = comp_match.as_str();
                } else {
                    compare = "=";
                }

                let number = captures.get(2).unwrap().as_str();

                if let Ok(number) = number.parse::<u16>() {
                    expr += &format!("{} {} {}", ident, compare, number);
                    value = "";
                } else {
                    return Err(ErrorBadRequest(format!(
                        "Expected number but got {}",
                        value
                    )));
                }
            }
        }

        Ok(Expr::cust_with_values(&expr, vec![value]))
    }
}
