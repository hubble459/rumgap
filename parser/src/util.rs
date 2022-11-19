use crabquery::{Selectable, Elements};
use regex::Regex;
use reqwest::Url;

use crate::parse_error::{Result, ParseError};

pub fn string_to_status(status: &str) -> bool {
    match status.to_lowercase().as_str() {
        "completed" | "dropped" | "finished" | "stopped" | "done" => false,
        _ => true,
    }
}

// pub fn first_attr(element: &Element, attrs: &Vec<&str>) -> Option<String> {
//     use crabquery::Element;
//
//     for attr in attrs {
//         if let Some(value) = element.attr(attr) {
//             return Some(value);
//         }
//     }
//     return None;
// }

///
/// Return the result of the first selector that matches anything
///
/// # Example
/// query = "a, p[example], p";
/// where body is
/// <div>
///     <p example>hello</p>
/// </div>
/// will only return 1 p
/// instead of 2
pub fn select<T>(element: &T, query: &str) -> Elements
where
    T: Selectable,
{
    let regex = Regex::new(r",\s*").unwrap();
    let queries = regex.split(query);
    for query in queries {
        let elements = element.select(query);
        if !elements.is_empty() {
            return elements.into();
        }
    }
    return vec![].into();
}

pub fn merge_attr_with_default(
    attr: &Option<&'static str>,
    default: Vec<&'static str>,
) -> Vec<&'static str> {
    if let Some(attr) = attr {
        return merge_vec_with_default(&Some(vec![attr]), default);
    } else {
        return default;
    }
}

pub fn merge_vec_with_default(
    attrs: &Option<Vec<&'static str>>,
    default: Vec<&'static str>,
) -> Vec<&'static str> {
    let mut all_attrs: Vec<&str> = vec![];

    if let Some(attrs) = attrs {
        for attr in attrs.iter() {
            all_attrs.push(attr);
        }
    }

    for attr in default {
        if !all_attrs.contains(&attr) {
            all_attrs.push(attr);
        }
    }

    return all_attrs;
}

pub fn get_hostname(url: &Url) -> Result<String> {
    let hostname = url.host_str().ok_or(ParseError::MissingHostname(url.to_string()))?;
    let match_sub_domain = Regex::new(r"^.+\.([^.]+\.[^.]+)$").unwrap();
    let hostname = match_sub_domain.replace(hostname, "$1").into_owned();

    Ok(hostname)
}
