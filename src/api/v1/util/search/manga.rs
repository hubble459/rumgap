use actix_web::error::ErrorBadRequest;
use migration::{Expr, SimpleExpr};

use crate::api::v1::{util::search::parse::Field, route::manga::NEXT_UPDATE_QUERY};

use super::{parse::Search, field::SearchField};


static SEARCH_FIELDS: phf::Map<&'static str, SearchField> = phf_map! {
    "title" => SearchField::Text("ARRAY_TO_STRING(manga.alt_titles, ', ') || ' ' || manga.title"),
    "description" => SearchField::Text("manga.description"),
    "url" => SearchField::Equals("manga.url"),
    "genres" => SearchField::Array("manga.genres"),
    "genre" => SearchField::Array("manga.genres"),
    "authors" => SearchField::Array("manga.authors"),
    "author" => SearchField::Array("manga.authors"),
    "last" => SearchField::Date("MAX(chapter.posted)"),
    "next" => SearchField::Date(NEXT_UPDATE_QUERY),
    "chapters" => SearchField::Number("COUNT(chapter.id)"),
};

pub fn lucene_filter(query: Search) -> actix_web::Result<SimpleExpr> {
    let with_fields: Vec<&Field> = query.iter().filter(|q| q.name.is_some()).collect();
    // TODO 13/12/2022: Use group_by when stable

    let mut expressions = vec![];
    for field in with_fields.into_iter() {
        let name = field.name.as_ref().unwrap();
        let name_key = SEARCH_FIELDS.get(name);
        if name_key.is_none() {
            return Err(ErrorBadRequest(format!(
                "Field with name '{}' is not allowed",
                name
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

    if !without_fields.is_empty() {
        let expr = Expr::cust_with_values(
            &format!(
                r#"
                ARRAY_TO_STRING(manga.genres, ', ')     || ' ' ||
                ARRAY_TO_STRING(manga.authors, ', ')    || ' ' ||
                ARRAY_TO_STRING(manga.alt_titles, ', ') || ' ' ||
                manga.description                       || ' ' ||
                manga.title                             ILIKE {}"#,
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
            &format!(
                r#"
                NOT (ARRAY_TO_STRING(manga.genres, ', ')|| ' ' ||
                ARRAY_TO_STRING(manga.authors, ', ')    || ' ' ||
                ARRAY_TO_STRING(manga.alt_titles, ', ') || ' ' ||
                manga.description                       || ' ' ||
                manga.title                             ILIKE {})"#,
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