use thiserror::Error;

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Network error, status = {0}")]
    NetworkError(reqwest::StatusCode),
    #[error(transparent)]
    NetworkErrorUnknown(#[from] reqwest::Error),
    #[error("Cloudflare's I'm Under Attack Mode")]
    CloudflareIUAM,
    #[error("HTML could not be parsed")]
    BadHTML,
    #[error("No url found in element '{:?}' with attrs: {:?}", .0, .1)]
    NoUrlFound(Option<String>, Vec<&'static str>),
    #[error("Failed to make '{0}' absolute")]
    FailedToMakeAbsolute(String),
    #[error("Parser does not accept this URL: {0}")]
    NotAccepted(String),
    #[error("No parser found that supports {0}")]
    NoParserFound(String),
    #[error("Missing hostname in url: {0}")]
    MissingHostname(String),
    #[error("Missing query: {0}")]
    MissingQuery(&'static str),
    // Manga
    #[error("Missing manga title")]
    MissingMangaTitle,

    // Chapters
    #[error("Missing chapter title")]
    MissingChapterTitle,
    #[error("Missing chapter href")]
    MissingChapterHref,
    #[error("Invalid chapter url: {0}")]
    InvalidChapterUrl(String),

    // Images
    #[error("No images")]
    MissingImages,

    // Search
    #[error("Missing search title")]
    MissingSearchTitle,
    #[error("Missing search href")]
    MissingSearchHref,
    #[error("Searching is not implemented for this parser")]
    SearchNotImplemented,
    #[error("Missing hostnames to search on")]
    SearchMissingHostnames,
    #[error("Invalid search url: {0}")]
    InvalidSearchUrl(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),

    #[error("An error occurred! {0}")]
    OtherStr(&'static str),
}

pub type Result<T> = core::result::Result<T, ParseError>;
