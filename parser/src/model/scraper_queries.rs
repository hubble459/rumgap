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
            search: Default::default(),
            hostnames: Default::default(),
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
            href: "a",
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

impl Default for GenericQueryImages {
    fn default() -> Self {
        Self {
            image: "//img",
            image_attrs: Default::default(),
        }
    }
}

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

pub struct GenericQueryMangaChapter {
    pub base: &'static str,
    pub href: &'static str,
    pub href_attr: Option<&'static str>,
    pub title: Option<&'static str>,
    pub title_attr: Option<&'static str>,
    pub posted: Option<&'static str>,
    pub posted_attr: Option<&'static str>,
    pub number: Option<&'static str>,
    pub number_attr: Option<&'static str>,
}

pub struct GenericQueryImages {
    pub image: &'static str,
    pub image_attrs: Option<Vec<&'static str>>,
}

pub struct GenericQuerySearch {
    pub url: &'static str,
    pub href: &'static str,
    pub href_attr: Option<&'static str>,
    pub title: &'static str,
    pub title_attr: Option<&'static str>,
    pub image: Option<&'static str>,
    pub image_attrs: Option<Vec<&'static str>>,
    pub updated: Option<&'static str>,
    pub updated_attr: Option<&'static str>,
    pub encode: bool,
    pub hostnames: Option<Vec<&'static str>>,
}
