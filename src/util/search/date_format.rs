use chrono::{NaiveDateTime, Utc};
use regex::Regex;
use tonic::Status;

pub struct DateFormat(pub NaiveDateTime);

lazy_static! {
    static ref DATE_FMT_REGEX: Regex = Regex::new(r"(\d+)([hdwmy])?").unwrap();
}

impl DateFormat {
    /// Try to parse a &str to a date
    ///
    /// String can be 1m, 2d etc.
    /// Formats are:
    /// h => hour
    /// d => day
    /// w => week
    /// m => month
    /// y => year
    /// Default format is "d"
    pub fn try_from(value: &str, in_future: bool) -> Result<Self, Status> {
        let captures = DATE_FMT_REGEX
            .captures(value)
            .ok_or(Status::invalid_argument(format!(
                "Expected date format but got {}",
                value
            )))?;

        let amount = captures.get(1).unwrap().as_str();
        let amount: i64 = amount
            .parse()
            .map_err(|_| Status::invalid_argument(format!("Expected number but got {}", amount)))?;

        let date_type = if let Some(date_type) = captures.get(2) {
            date_type.as_str()
        } else {
            "d"
        };

        let millis = Utc::now().timestamp_millis();
        let change = match date_type {
            "h" => 3600000 * amount,
            "d" => 86400000 * amount,
            "w" => 604800000 * amount,
            "m" => 2629800000 * amount,
            "y" => 31536000000 * amount,
            _ => unreachable!("Regex does not match anything else"),
        };
        let millis = if in_future {
            millis + change
        } else {
            let millis = millis - change;
            if millis.is_negative() {
                millis
            } else {
                millis
            }
        };

        let date_time = NaiveDateTime::from_timestamp_millis(millis);
        // TODO 14/12/2022: proper error
        let date_time = date_time.unwrap();
        debug!("{}", date_type.to_string());
        Ok(Self(date_time))
    }
}
