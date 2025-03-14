use migration::{Expr, SimpleExpr};
use tonic::Status;

use self::field::SearchField;
use self::parse::Search;
use super::search::parse::Field;

pub mod date_format;
pub mod field;
pub mod manga;
pub mod parse;

/// Search parser for generating a SeaORM query
pub fn lucene_filter(map: &phf::Map<&'static str, SearchField>, query: Search) -> Result<SimpleExpr, Status> {
    let with_fields: Vec<&Field> = query.iter().filter(|q| q.name.is_some()).collect();
    // TODO 13/12/2022: Use group_by when stable

    let mut expressions = vec![];
    for field in with_fields.into_iter() {
        let name = field.name.as_ref().unwrap();
        let name_key = map.get(name);
        if name_key.is_none() {
            return Err(Status::invalid_argument(format!(
                "Field with name '{name}' does not exist"
            )));
        }

        let expr = name_key.cloned().unwrap().into_expression(&field.value, field.exclude);
        match expr {
            Ok(expr) => expressions.push(expr),
            Err(e) => return Err(e),
        }
    }

    let without_fields: Vec<String> = query
        .iter()
        .filter(|q| q.name.is_none() && !q.exclude)
        .map(|field| format!("%{}%", field.value))
        .collect();

    let all_fields = map.get("*").unwrap().to_string();

    if !without_fields.is_empty() {
        let expr = Expr::cust_with_values(
            format!(
                "{} ILIKE {}",
                all_fields,
                (0..without_fields.len())
                    .enumerate()
                    .map(|(i, _)| format!("${}", i + 1))
                    .collect::<Vec<String>>()
                    .join(" || ")
            ),
            without_fields,
        );

        expressions.push(expr);
    }

    let exclude_fields: Vec<String> = query
        .iter()
        .filter(|q| q.name.is_none() && q.exclude)
        .map(|field| format!("%{}%", field.value))
        .collect();

    if !exclude_fields.is_empty() {
        let expr = Expr::cust_with_values(
            format!(
                "NOT {} {}",
                all_fields,
                (0..exclude_fields.len())
                    .enumerate()
                    .map(|(i, _)| format!("${}", i + 1))
                    .collect::<Vec<String>>()
                    .join(" || ")
            ),
            exclude_fields,
        );

        expressions.push(expr);
    }

    if expressions.is_empty() {
        return Ok(Expr::cust("true"));
    }
    let first = expressions.first().unwrap().clone();

    let expression = expressions
        .into_iter()
        .skip(1)
        .fold(first, |total, expr| total.and(expr));

    Ok(expression)
}
