use crabquery::Element;

pub fn string_to_status(status: &str) -> bool {
    match status.to_lowercase().as_str() {
        "completed" | "dropped" | "finished" | "stopped" | "done" => false,
        _ => true,
    }
}

pub fn first_attr(element: &Element, attrs: &Vec<&str>) -> Option<String> {
    for attr in attrs {
        if let Some(value) = element.attr(attr) {
            return Some(value);
        }
    }
    return None;
}

pub fn merge_attr_with_default(attr: &Option<&'static str>, default: Vec<&'static str>) -> Vec<&'static str> {
    if let Some(attr) = attr {
        return merge_vec_with_default(&Some(vec![attr]), default);
    } else {
        return default;
    }
}

pub fn merge_vec_with_default(attrs: &Option<Vec<&'static str>>, default: Vec<&'static str>) -> Vec<&'static str> {
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