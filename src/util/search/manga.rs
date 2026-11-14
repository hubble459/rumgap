use migration::SimpleExpr;
use tonic::Status;

use super::field::SearchField;
use super::parse::Search;
use crate::service::v1::manga::NEXT_UPDATE_QUERY;

// Text fields match any of the manga's sources, since each site has its own details.
const SELECT_MANGA_ALL: &str = r#"(SELECT string_agg(
    ARRAY_TO_STRING(s.genres, ', ')     || ' ' ||
    ARRAY_TO_STRING(s.authors, ', ')    || ' ' ||
    ARRAY_TO_STRING(s.alt_titles, ', ') || ' ' ||
    s.description                       || ' ' ||
    s.title, ' ') FROM manga_source s WHERE s.manga_id = manga.id)"#;

static SEARCH_FIELDS: phf::Map<&'static str, SearchField> = phf_map! {
    "id" => SearchField::Number("manga.id"),
    "title" => SearchField::Text("(SELECT string_agg(ARRAY_TO_STRING(s.alt_titles, ', ') || ' ' || s.title, ' ') FROM manga_source s WHERE s.manga_id = manga.id)"),
    "description" => SearchField::Text("(SELECT string_agg(s.description, ' ') FROM manga_source s WHERE s.manga_id = manga.id)"),
    // Resolves through the source `index_manga` shows each manga through.
    "url" => SearchField::Equals("manga_source.url"),
    "origin" => SearchField::Text("manga.origin"),
    "host" => SearchField::Text("(SELECT string_agg(s.hostname, ' ') FROM manga_source s WHERE s.manga_id = manga.id)"),
    "genres" => SearchField::Array("(SELECT array_agg(g) FROM manga_source s, unnest(s.genres) g WHERE s.manga_id = manga.id)"),
    "genre" => SearchField::Array("(SELECT array_agg(g) FROM manga_source s, unnest(s.genres) g WHERE s.manga_id = manga.id)"),
    "authors" => SearchField::Array("(SELECT array_agg(a) FROM manga_source s, unnest(s.authors) a WHERE s.manga_id = manga.id)"),
    "author" => SearchField::Array("(SELECT array_agg(a) FROM manga_source s, unnest(s.authors) a WHERE s.manga_id = manga.id)"),
    "last" => SearchField::Date("MAX(chapter.posted)", false),
    "next" => SearchField::Date(NEXT_UPDATE_QUERY, true),
    "chapter" => SearchField::Number("COUNT(chapter.id)"),
    "chapters" => SearchField::Number("COUNT(chapter.id)"),
    "progress" => SearchField::Number("reading.progress"),
    "reading" => SearchField::Number("reading.progress"),
    "*" => SearchField::Text(SELECT_MANGA_ALL),
};

pub fn lucene_filter(query: Search) -> Result<SimpleExpr, Status> {
    super::lucene_filter(&SEARCH_FIELDS, query)
}
