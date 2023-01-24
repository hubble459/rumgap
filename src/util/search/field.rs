use migration::{Expr, SimpleExpr};
use regex::Regex;
use tonic::Status;

use super::date_format::DateFormat;

lazy_static! {
    static ref SEARCH_DATE_REGEX: Regex = Regex::new(r"([<>]=?)?(.+)").unwrap();
}

#[derive(Debug, Clone)]
pub enum SearchField {
    Array(&'static str),
    Text(&'static str),
    Date(&'static str, bool),
    Equals(&'static str),
    Number(&'static str),
}

impl ToString for SearchField {
    fn to_string(&self) -> String {
        match self {
            SearchField::Array(s) => s.to_string(),
            SearchField::Text(s) => s.to_string(),
            SearchField::Date(s, _) => s.to_string(),
            SearchField::Equals(s) => s.to_string(),
            SearchField::Number(s) => s.to_string(),
        }
    }
}

impl SearchField {
    pub fn into_expression(self, value: &str, exclude: bool) -> Result<SimpleExpr, Status> {
        let mut wild = false;
        let mut expr = String::new();

        if exclude {
            expr += "NOT ";
        }

        match self {
            SearchField::Array(ident) => {
                expr += &format!("ARRAY_TO_STRING({}, ', ') ILIKE $1", ident);
                wild = true;
            }
            SearchField::Text(ident) => {
                expr += &format!("{} ILIKE $1", ident);
                wild = true;
            }
            SearchField::Date(ident, future) => {
                let captures = SEARCH_DATE_REGEX.captures(&value).unwrap();
                let compare: String;

                if let Some(comp_match) = captures.get(1) {
                    let cmp = comp_match.as_str();
                    if cmp.ends_with("=") {
                        compare = cmp.to_owned();
                    } else {
                        compare = cmp.to_owned() + "=";
                    }
                } else {
                    compare = String::from(">=");
                }

                let date = captures.get(2).unwrap().as_str();
                let date = DateFormat::try_from(date, future)?;

                return Ok(Expr::cust_with_values(&format!("{} {} $1", ident, compare), vec![date.0]));
            }
            SearchField::Equals(ident) => {
                expr += &format!("{} = $1", ident);
            }
            SearchField::Number(ident) => {
                let captures = SEARCH_DATE_REGEX
                    .captures(&value)
                    .ok_or(Status::invalid_argument(format!(
                        "Expected number but got {}",
                        value
                    )))?;
                let compare;

                if let Some(comp_match) = captures.get(1) {
                    compare = comp_match.as_str();
                } else {
                    compare = "=";
                }

                let number = captures.get(2).unwrap().as_str();

                if let Ok(number) = number.parse::<u16>() {
                    return Ok(Expr::cust_with_values(&format!("{} {} $1", ident, compare), vec![number]));
                } else {
                    return Err(Status::invalid_argument(format!(
                        "Expected number but got {}",
                        value
                    )));
                }
            }
        }

        let value = if wild {
            format!("%{}%", value)
        } else {
            value.to_string()
        };

        Ok(Expr::cust_with_values(&expr, vec![value]))
    }
}
