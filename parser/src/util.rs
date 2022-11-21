use chrono::{DateTime, Duration, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use crabquery::{Element, Elements, Selectable};
use regex::{Regex, Captures};
use reqwest::Url;

use crate::parse_error::{ParseError, Result};

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

pub fn select_first<T>(element: &T, query: &str) -> Option<Element>
where
    T: Selectable,
{
    let elements = select(element, query);
    return elements.elements.first().cloned();
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
    let hostname = url
        .host_str()
        .ok_or(ParseError::MissingHostname(url.to_string()))?;
    let match_sub_domain = Regex::new(r"^.+\.([^.]+\.[^.]+)$").unwrap();
    let hostname = match_sub_domain.replace(hostname, "$1").into_owned();

    Ok(hostname)
}

pub fn select_element_fallback<T>(
    root: &T,
    query: Option<&str>,
    fallback_element: Option<Element>,
) -> Option<Element>
where
    T: Selectable,
{
    if let Some(query) = query {
        let elements = select(root, query);
        let element = elements.elements.first().cloned();
        if element.is_none() {
            fallback_element
        } else {
            element
        }
    } else {
        fallback_element
    }
}

pub fn select_text_or_attr(element: Element, attr: Option<&str>) -> Option<String> {
    let text = if let Some(attr) = attr {
        element.attr(attr).or(element.text())
    } else {
        element.text()
    };

    text.map(|t| t.trim().to_owned())
}

pub fn select_fallback<T>(
    root: &T,
    query: Option<&str>,
    query_attr: Option<&str>,
    fallback_element: Option<Element>,
) -> Option<String>
where
    T: Selectable,
{
    let element = select_element_fallback(root, query, fallback_element);
    if let Some(element) = element {
        select_text_or_attr(element, query_attr)
    } else {
        None
    }
}

lazy_static! {
    static ref CLEAN_DATE: Regex = Regex::new(r"[^\w\d:.+\-]+").unwrap();
    static ref CLEAN_DATE_2: Regex = Regex::new(r"-{2,}").unwrap();
    static ref ORDINAL_NUMBER: Regex = Regex::new(r"(\d)(nd|st|rd|th)").unwrap();
    static ref DIGITS_ONLY: Regex = Regex::new(r"^\d+$").unwrap();
    static ref HAS_DIGITS: Regex = Regex::new(r"\d+").unwrap();
    static ref NONE_LETTER: Regex = Regex::new(r"\W").unwrap();
    static ref RELATIVE_DATE: Regex = Regex::new(r"(\d+)\s*(\w\w?)").unwrap();
}

/// Selects "1 year ago" -> "1y"

const STRING_FOR_CURRENT_DATE: [&str; 6] = ["now", "latest", "hot", "today", "current", "while"];

const DEFAULT_DATE_FORMATS: [&str; 18] = [
    // 2022-01-30T09:10:11.123Z
    "%Y-%m-%dT%H:%M:%S%.fZ",
    // 2022-01-30T09:10:11.123+0800
    "%Y-%m-%dT%H:%M:%S%.f%z",
    // 2022-01-30T09:10:11+0800
    "%Y-%m-%dT%H:%M:%S%z",
    // 2022-01-30T09:10:11Z
    "%Y-%m-%dT%H:%M:%SZ",
    // 2022-01-30T09:10:11
    "%Y-%m-%dT%H:%M:%S",
    // Juli 30 22 - 09:10
    "%B-%d-%y-%H:%M",
    // Juli 30 2022 09:10
    "%B-%d-%Y-%H:%M",
    // Oct 30 22 09:10:11
    "%b-%d-%y-%H:%M:%S",
    // Juli-30,22 09:10:11
    "%B-%d-%y-%H:%M:%S",
    // Oct 30 09:10
    "%b-%d-%H:%M",
    // 30 Juli 09:10
    "%d-%B-%H:%M",
    // 30 Oct 09:10
    "%d-%b-%H:%M",
    // Juli 30 2022
    "%B-%d-%Y",
    // Oct 30 2022
    "%b-%d-%Y",
    // Oct 30 22
    "%b-%d-%y",
    // 30 Juli 2022
    "%d-%B-%Y",
    // 2022.12.30
    "%Y.%m.%d",
    // 30 01 2022
    "%d-%m-%Y",
];

pub fn try_parse_date(date: &str) -> Option<DateTime<Utc>> {
    if date.is_empty() {
        return None;
    }
    let date = date.trim();

    // Check if epoch millis [digits only]
    if DIGITS_ONLY.is_match(date) {
        let millis: i64 = date.parse().unwrap_or(-1);
        if millis == -1 {
            return None;
        }
        return NaiveDateTime::from_timestamp_millis(millis)
            .map(|datetime| DateTime::<Utc>::from_utc(datetime, Utc));
    }

    let now = Utc::now();

    // Check if text only
    if !HAS_DIGITS.is_match(date) {
        let date = NONE_LETTER.replace_all(&date, "").to_ascii_lowercase();

        for current_string in STRING_FOR_CURRENT_DATE {
            if date.contains(current_string) {
                return Some(now);
            }
        }
        if date.contains("yesterday") {
            return now.checked_sub_signed(Duration::days(1));
        }
        if date.contains("week") {
            return now.checked_sub_signed(Duration::weeks(1));
        }
        if date.contains("month") {
            return now.checked_sub_months(Months::new(1));
        }
        if date.contains("year") {
            return now.checked_sub_signed(Duration::days(365));
        }
        return None;
    }

    // Check if date format (multiple digits)
    if HAS_DIGITS.find_iter(date).count() > 1 {
        let date = CLEAN_DATE.replace_all(date, "-").into_owned();
        let date = CLEAN_DATE_2.replace_all(&date, "-");
        let date = &ORDINAL_NUMBER.replace_all(&date.into_owned(), |cap: &Captures| cap[1].to_owned()).into_owned();
        for format in DEFAULT_DATE_FORMATS {
            let datetime = NaiveDateTime::parse_from_str(date, format);
            if let Ok(date) = datetime {
                return Some(Utc.from_utc_datetime(&date));
            } else if let Err(e) = datetime {
                // If missing time
                if e.kind() == chrono::format::ParseErrorKind::NotEnough {
                    // Parse only date
                    let date = NaiveDate::parse_from_str(date, format);
                    if let Ok(date) = date {
                        // Return date time with default time
                        return Some(Utc.from_utc_datetime(&date.and_time(NaiveTime::default())));
                    }
                }
            }
        }
    }

    // Check if relative
    // e.g. "1 year ago"
    let binding = date.to_ascii_lowercase();
    let captures = RELATIVE_DATE.captures(&binding);
    if let Some(captures) = captures {
        // Assume that it always is [number][type] ago
        // like 1 year ago
        let amount: i64 = captures.get(1).unwrap().as_str().parse().unwrap_or(1);
        let rel_type = captures.get(2).unwrap().as_str();

        // Minutes
        if rel_type == "mi" {
            return Some(now - Duration::minutes(amount));
        }

        let rel_type = rel_type.chars().nth(0).unwrap();

        return match rel_type {
            's' => Some(now - Duration::seconds(amount)),
            'h' => Some(now - Duration::hours(amount)),
            'd' => Some(now - Duration::days(amount)),
            'w' => Some(now - Duration::weeks(amount)),
            'm' => Some(now - Months::new(amount as u32)),
            'y' => Some(now - Duration::days(365 * amount)),
            _ => None,
        };
    }

    // No date detected
    None
}
