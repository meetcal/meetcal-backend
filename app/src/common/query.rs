use crate::AppError;
use serde::{Deserialize, Deserializer};

pub const MAX_NAME_LIST_LEN: usize = 100;
pub const MAX_SAVED_SESSION_ATHLETE_NAMES: usize = 64;

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

pub fn deserialize_csv_or_repeated<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let values = match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(value) => vec![value],
        OneOrMany::Many(values) => values,
    };

    Ok(values
        .into_iter()
        .flat_map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .collect())
}

pub fn require_non_empty(field: &str, value: &str) -> Result<(), AppError> {
    if value.trim().is_empty() {
        return Err(AppError::Validation(format!("{field} is required")));
    }
    Ok(())
}

/// Builds a `LIKE`/`ILIKE` "contains" pattern from caller input.
///
/// `%` and `_` are wildcards and `\` is Postgres' default escape character, so
/// a raw `format!("%{value}%")` lets a query of `%` or `_` match every row
/// instead of the rows containing that character. Escaping them keeps the
/// pattern a literal substring search.
pub fn like_contains_pattern(value: &str) -> String {
    let mut pattern = String::with_capacity(value.len() + 2);
    pattern.push('%');
    for character in value.chars() {
        if matches!(character, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push('%');
    pattern
}

/// True only for a real calendar day written as `YYYY-MM-DD`.
///
/// Date columns in `lifting_results` are text, so a malformed cutoff is not a
/// SQL error -- it silently compares as a string and returns a nonsense window.
/// The endpoints that take a date share this one rule rather than each deciding
/// what a date is.
pub fn is_valid_iso_date(value: &str) -> bool {
    let mut parts = value.split('-');
    let (Some(year), Some(month), Some(day), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    if year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return false;
    }
    let (Ok(year), Ok(month), Ok(day)) = (
        year.parse::<u32>(),
        month.parse::<u32>(),
        day.parse::<u32>(),
    ) else {
        return false;
    };
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

/// Validator form of [`is_valid_iso_date`] for an optional query parameter.
/// Absent passes; present-but-malformed is a 400.
pub fn require_iso_date(field: &str, value: Option<&str>) -> Result<(), AppError> {
    match value {
        Some(value) if !is_valid_iso_date(value) => Err(AppError::Validation(format!(
            "{field} must be a valid YYYY-MM-DD date"
        ))),
        _ => Ok(()),
    }
}

pub fn require_name_list(names: &[String]) -> Result<(), AppError> {
    require_non_empty("names", names.first().map(String::as_str).unwrap_or(""))?;
    if names.len() > MAX_NAME_LIST_LEN {
        return Err(AppError::Validation(format!(
            "names exceeds the {MAX_NAME_LIST_LEN}-name limit"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_oversized_name_lists() {
        assert!(require_name_list(&[]).is_err());
        assert!(require_name_list(&["Ada".to_string()]).is_ok());
        let oversized: Vec<String> = (0..=MAX_NAME_LIST_LEN)
            .map(|index| format!("name-{index}"))
            .collect();
        assert!(require_name_list(&oversized).is_err());
    }

    #[test]
    fn like_patterns_escape_wildcards() {
        assert_eq!(like_contains_pattern("Ada"), "%Ada%");
        // A bare wildcard matched every row before it was escaped.
        assert_eq!(like_contains_pattern("%"), "%\\%%");
        assert_eq!(like_contains_pattern("_"), "%\\_%");
        assert_eq!(like_contains_pattern("a%b_c\\d"), "%a\\%b\\_c\\\\d%");
    }

    #[test]
    fn validates_real_iso_dates() {
        assert!(is_valid_iso_date("2024-02-29"));
        assert!(is_valid_iso_date("2026-08-23"));
        assert!(!is_valid_iso_date("2025-02-29"));
        assert!(!is_valid_iso_date("2026-13-01"));
        assert!(!is_valid_iso_date("not-a-date"));
        assert!(!is_valid_iso_date(""));
    }

    #[test]
    fn optional_dates_pass_when_absent_and_fail_when_malformed() {
        assert!(require_iso_date("cutoff_date", None).is_ok());
        assert!(require_iso_date("cutoff_date", Some("2025-06-13")).is_ok());
        assert!(require_iso_date("cutoff_date", Some("2025-6-13")).is_err());
        assert!(require_iso_date("cutoff_date", Some("yesterday")).is_err());
    }

    #[test]
    fn rejects_blank_fields() {
        assert!(require_non_empty("club", "").is_err());
        assert!(require_non_empty("club", "   ").is_err());
        assert!(require_non_empty("club", "Ohio").is_ok());
    }
}
