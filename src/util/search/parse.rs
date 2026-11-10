#[derive(Debug, PartialEq)]
pub struct Field {
    pub name: Option<String>,
    pub value: String,
    pub exclude: bool,
    pub exact: bool,
}

#[derive(Debug, PartialEq)]
pub struct Search(pub Vec<Field>);

impl std::ops::Deref for Search {
    type Target = Vec<Field>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<&str> for Search {
    fn from(v: &str) -> Self {
        let mut inside = false;

        let mut name = None;
        let mut value = String::new();
        let mut exclude = false;
        let mut exact = false;

        let mut fields = vec![];

        for char in v.chars() {
            match char {
                ' ' => {
                    if !inside {
                        fields.push(Field {
                            exclude,
                            name,
                            value,
                            exact,
                        });

                        name = None;
                        value = String::new();
                        exclude = false;
                        exact = false;
                    } else {
                        value += " ";
                    }
                }
                ':' => {
                    if !inside {
                        name = Some(value);
                        value = String::new();
                    }
                }
                '-' => {
                    if !inside && value.is_empty() {
                        exclude = true;
                    } else {
                        value += "-";
                    }
                }
                '"' => {
                    inside = !inside;
                    if inside {
                        exact = true;
                    }
                }
                c => {
                    value += &c.to_string();
                }
            }
        }

        if !value.is_empty() {
            fields.push(Field {
                exclude,
                name,
                value,
                exact,
            });
        }

        Self(fields)
    }
}

impl From<String> for Search {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

#[test]
fn parse_search_query() {
    let query: Search = "owo".into();
    assert_eq!(
        query.first().unwrap(),
        &Field {
            exclude: false,
            exact: false,
            name: None,
            value: String::from("owo"),
        }
    );

    let query: Search = "-owo".into();
    assert_eq!(
        query.first().unwrap(),
        &Field {
            exclude: true,
            exact: false,
            name: None,
            value: String::from("owo")
        }
    );

    let query: Search = "type:owo".into();
    assert_eq!(
        query.first().unwrap(),
        &Field {
            exclude: false,
            exact: false,
            name: Some(String::from("type")),
            value: String::from("owo")
        }
    );

    let query: Search = "-type:owo".into();
    assert_eq!(
        query.first().unwrap(),
        &Field {
            exclude: true,
            exact: false,
            name: Some(String::from("type")),
            value: String::from("owo")
        }
    );

    let query: Search = r#""owo uwu""#.into();
    assert_eq!(
        query.first().unwrap(),
        &Field {
            exclude: false,
            exact: true,
            name: None,
            value: String::from("owo uwu")
        }
    );

    let query: Search = r#"type:"owo uwu""#.into();
    assert_eq!(
        query.first().unwrap(),
        &Field {
            exclude: false,
            exact: true,
            name: Some(String::from("type")),
            value: String::from("owo uwu")
        }
    );

    let query: Search = r#"-type:"owo uwu""#.into();
    assert_eq!(
        query.first().unwrap(),
        &Field {
            exclude: true,
            exact: true,
            name: Some(String::from("type")),
            value: String::from("owo uwu")
        }
    );
}
