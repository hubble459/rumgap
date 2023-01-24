use migration::SimpleExpr;
use tonic::Status;

use super::field::SearchField;
use super::parse::Search;

const SELECT_MANGA_ALL: &str = r#"
ARRAY_TO_STRING(manga.genres, ', ')     || ' ' ||
ARRAY_TO_STRING(manga.authors, ', ')    || ' ' ||
ARRAY_TO_STRING(manga.alt_titles, ', ') || ' ' ||
manga.description                       || ' ' ||
manga.title                             ILIKE"#;

static SEARCH_FIELDS: phf::Map<&'static str, SearchField> = phf_map! {
    "title" => SearchField::Text("ARRAY_TO_STRING(manga.alt_titles, ', ') || ' ' || manga.title"),
    "description" => SearchField::Text("manga.description"),
    "url" => SearchField::Equals("manga.url"),
    "genres" => SearchField::Array("manga.genres"),
    "genre" => SearchField::Array("manga.genres"),
    "authors" => SearchField::Array("manga.authors"),
    "author" => SearchField::Array("manga.authors"),
    "last" => SearchField::Date("MAX(chapter.posted)", false),
    "next" => SearchField::Date(SELECT_MANGA_ALL, true),
    "chapter" => SearchField::Number("COUNT(chapter.id)"),
    "chapters" => SearchField::Number("COUNT(chapter.id)"),
    "progress" => SearchField::Number("reading.progress"),
    "updated" => SearchField::Date("reading.updated_at", false),
    "created" => SearchField::Date("reading.created_at", false),
    "*" => SearchField::Text(SELECT_MANGA_ALL),
};

pub fn lucene_filter(query: Search) -> Result<SimpleExpr, Status> {
    super::lucene_filter(&SEARCH_FIELDS, query)
}
