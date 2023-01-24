use migration::{SimpleExpr, Expr};
use tonic::Status;

use super::search::parse::Field;

use self::{parse::Search, field::SearchField};

pub mod field;
pub mod manga;
pub mod reading;
pub mod parse;
pub mod date_format;

pub fn lucene_filter(map: &phf::Map<&'static str, SearchField>, query: Search) -> Result<SimpleExpr, Status> {
    let with_fields: Vec<&Field> = query.iter().filter(|q| q.name.is_some()).collect();
    // TODO 13/12/2022: Use group_by when stable

    let mut expressions = vec![];
    for field in with_fields.into_iter() {
        let name = field.name.as_ref().unwrap();
        let name_key = map.get(name);
        if name_key.is_none() {
            return Err(Status::invalid_argument(format!(
                "Field with name '{}' does not exist",
                name
            )));
        }

        let expr = name_key
            .cloned()
            .unwrap()
            .into_expression(&field.value, field.exclude);
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
            &format!(
                "{} {}",
                all_fields, (0..without_fields.len())
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
            &format!(
                "NOT {} {}",
                all_fields, (0..exclude_fields.len())
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
        return Ok(Expr::val(1).eq(1));
    }
    let first = expressions.first().unwrap().clone();

    let expression = expressions
        .into_iter()
        .skip(1)
        .fold(first, |total, expr| total.and(expr));

    println!(
        "Filter {}",
        migration::Query::select()
            .and_where(expression.clone())
            .to_owned()
            .to_string(migration::PostgresQueryBuilder)
    );

    Ok(expression)
}