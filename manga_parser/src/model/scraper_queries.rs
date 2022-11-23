#[derive(Clone, Debug)]
pub struct GenericQuery {
    pub manga: GenericQueryManga,
    pub images: GenericQueryImages,
    pub search: Option<GenericQuerySearch>,
    pub hostnames: Vec<&'static str>,
    pub date_formats: Vec<&'static str>,
}

impl Default for GenericQuery {
    fn default() -> Self {
        Self {
            manga: Default::default(),
            images: Default::default(),
            hostnames: Default::default(),
            search: None,
            date_formats: Default::default(),
        }
    }
}

impl Default for GenericQueryManga {
    fn default() -> Self {
        Self {
            title: "h1",
            title_attr: Default::default(),
            description: Default::default(),
            description_attr: Default::default(),
            cover: Default::default(),
            cover_attrs: Default::default(),
            is_ongoing: Default::default(),
            is_ongoing_attr: Default::default(),
            alt_titles: Default::default(),
            alt_titles_attr: Default::default(),
            authors: Default::default(),
            authors_attr: Default::default(),
            genres: Default::default(),
            genres_attr: Default::default(),
            chapter: Default::default(),
        }
    }
}

impl Default for GenericQueryMangaChapter {
    fn default() -> Self {
        Self {
            base: "ul, ol",
            href: Default::default(),
            href_attr: Default::default(),
            title: Default::default(),
            title_attr: Default::default(),
            posted: Default::default(),
            posted_attr: Default::default(),
            number: Default::default(),
            number_attr: Default::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct GenericQueryManga {
    pub title: &'static str,
    pub title_attr: Option<&'static str>,
    pub description: Option<&'static str>,
    pub description_attr: Option<&'static str>,
    pub cover: Option<&'static str>,
    pub cover_attrs: Option<Vec<&'static str>>,
    pub is_ongoing: Option<&'static str>,
    pub is_ongoing_attr: Option<&'static str>,
    pub alt_titles: Option<&'static str>,
    pub alt_titles_attr: Option<&'static str>,
    pub authors: Option<&'static str>,
    pub authors_attr: Option<&'static str>,
    pub genres: Option<&'static str>,
    pub genres_attr: Option<&'static str>,
    pub chapter: GenericQueryMangaChapter,
}

#[derive(Clone, Debug)]
pub struct GenericQueryMangaChapter {
    pub base: &'static str,
    /// if None, [base] will be used
    pub href: Option<&'static str>,
    pub href_attr: Option<&'static str>,
    pub title: Option<&'static str>,
    pub title_attr: Option<&'static str>,
    pub posted: Option<&'static str>,
    pub posted_attr: Option<&'static str>,
    pub number: Option<&'static str>,
    pub number_attr: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub struct GenericQueryImages {
    pub image: &'static str,
    pub image_attrs: Option<Vec<&'static str>>,
}

impl Default for GenericQueryImages {
    fn default() -> Self {
        Self {
            image: "img",
            image_attrs: Default::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct GenericQuerySearch {
    /// Like `/search?q=[query]` or `/search/[query]`
    ///
    /// Will be translated to https://<hostname><path>
    pub path: &'static str,
    pub base: &'static str,
    pub href: Option<&'static str>,
    pub href_attr: Option<&'static str>,
    pub title: Option<&'static str>,
    pub title_attr: Option<&'static str>,
    pub cover: Option<&'static str>,
    pub cover_attrs: Option<Vec<&'static str>>,
    pub posted: Option<&'static str>,
    pub posted_attr: Option<&'static str>,
    pub encode: bool,
    pub hostnames: Option<Vec<&'static str>>,
}

impl Default for GenericQuerySearch {
    fn default() -> Self {
        Self {
            base: "",
            path: "",
            href: Default::default(),
            href_attr: Default::default(),
            title: Default::default(),
            title_attr: Default::default(),
            cover: Default::default(),
            cover_attrs: Default::default(),
            posted: Default::default(),
            posted_attr: Default::default(),
            encode: true,
            hostnames: Default::default(),
        }
    }
}
