use crate::AppError;
use serde::{Deserialize, Deserializer};

pub const MAX_NAME_LIST_LEN: usize = 100;
/// Longest single name in a name list, in UTF-8 bytes (after trimming). The
/// longest real lifter names run to ~60 characters; 400 bytes is still at
/// least 100 characters of any script. The cap bounds the per-request
/// `normalize_name` work and the `= ANY($1)` array alongside
/// [`MAX_NAME_LIST_LEN`]. Like the list cap it is not version-gated: name
/// lists fail closed for every client.
pub const MAX_NAME_LEN: usize = 400;
/// Worst-case JSON bytes per decoded UTF-8 byte: a one-byte control character
/// sent as a `\u00XX` escape. (A four-byte astral character sent as a
/// `\uXXXX\uXXXX` surrogate pair is 12 bytes, only 3 per decoded byte.)
pub const MAX_JSON_BYTES_PER_UTF8_BYTE: usize = 6;
/// Worst-case JSON bytes per decoded character: an astral character sent as a
/// surrogate-pair escape.
pub const MAX_JSON_BYTES_PER_CHAR: usize = 12;
/// Request-body ceiling for the `POST` name-list endpoints
/// (`/lifting-results/by-names`, `/recent`, `/bests`). A maximal valid body is
/// [`MAX_NAME_LIST_LEN`] names of [`MAX_NAME_LEN`] decoded bytes; with every
/// byte escaped by an ASCII-only serializer that is 240,000 bytes, plus quotes
/// and commas (~400 bytes) and the optional scalar fields (~100 bytes). 256 KiB
/// admits every body that can pass validation however it is encoded, and
/// rejects anything larger with `413` before it is buffered.
pub const NAME_LIST_BODY_LIMIT: usize = 256 * 1024;
const _: () = assert!(
    MAX_NAME_LIST_LEN * (MAX_NAME_LEN * MAX_JSON_BYTES_PER_UTF8_BYTE + 4) + 1024
        <= NAME_LIST_BODY_LIMIT
);
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

/// JSON body for the `POST` name-list endpoints.
///
/// `GET` callers pass `names` as a comma-separated query param, which cannot
/// carry a name that itself contains a comma and is bounded by URL length. The
/// `POST` form takes a real array; `cutoff_date` is ignored by endpoints that
/// have no date window.
#[derive(Debug, Deserialize)]
pub struct NameListBody {
    pub names: Vec<String>,
    #[serde(default)]
    pub cutoff_date: Option<String>,
    /// `/lifting-results/by-names` only: keep each athlete's most recent
    /// meet rows. Ignored by endpoints that have no per-name bound.
    #[serde(default)]
    pub latest_only: Option<bool>,
    /// `/lifting-results/by-names` only: at most this many rows per name.
    #[serde(default)]
    pub limit_per_name: Option<u32>,
}

/// Trims each name and drops blanks, matching what the CSV deserializer does
/// for `GET` so the two forms validate identically.
pub fn clean_name_list(names: Vec<String>) -> Vec<String> {
    names
        .into_iter()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
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

/// True only for a four-digit calendar year, the form the `date` text columns
/// start with. A `year` of `abc` would otherwise build `abc-01-01` and compare
/// as a string against every row.
pub fn is_valid_year(value: &str) -> bool {
    value.len() == 4 && value.bytes().all(|byte| byte.is_ascii_digit())
}

pub fn require_year(field: &str, value: &str) -> Result<(), AppError> {
    if is_valid_year(value) {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "{field} must be a four-digit year"
        )))
    }
}

/// Empty and oversized name lists, and lists holding an oversized name, fail
/// closed for every client. This is not version-gated: an empty list was never
/// a meaningful request, and the size caps bound the `= ANY($1)` array so one
/// call cannot pin a connection.
pub fn require_name_list(names: &[String]) -> Result<(), AppError> {
    require_non_empty("names", names.first().map(String::as_str).unwrap_or(""))?;
    if names.len() > MAX_NAME_LIST_LEN {
        return Err(AppError::Validation(format!(
            "names exceeds the {MAX_NAME_LIST_LEN}-name limit"
        )));
    }
    if names.iter().any(|name| name.len() > MAX_NAME_LEN) {
        return Err(AppError::Validation(format!(
            "each name must be at most {MAX_NAME_LEN} bytes"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_name_list_trims_and_drops_blanks_but_keeps_commas() {
        let cleaned = clean_name_list(vec![
            "  Alexander Nordstrom ".to_string(),
            "".to_string(),
            "   ".to_string(),
            "Nordstrom, Alexander".to_string(),
        ]);
        assert_eq!(cleaned, vec!["Alexander Nordstrom", "Nordstrom, Alexander"]);
    }

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
    fn rejects_a_name_longer_than_the_per_name_cap() {
        let at_cap = "a".repeat(MAX_NAME_LEN);
        assert!(require_name_list(std::slice::from_ref(&at_cap)).is_ok());
        let over_cap = "a".repeat(MAX_NAME_LEN + 1);
        assert!(require_name_list(&["Ada".to_string(), over_cap]).is_err());
        // Bytes, not characters: 200 two-byte characters is exactly the cap.
        let multibyte_at_cap = "\u{e9}".repeat(MAX_NAME_LEN / 2);
        assert!(require_name_list(std::slice::from_ref(&multibyte_at_cap)).is_ok());
        assert!(require_name_list(&[format!("{multibyte_at_cap}a")]).is_err());
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
    fn validates_four_digit_years() {
        assert!(is_valid_year("2026"));
        assert!(!is_valid_year("26"));
        assert!(!is_valid_year("20260"));
        assert!(!is_valid_year("abcd"));
        assert!(!is_valid_year(""));
        assert!(require_year("year", "1999").is_ok());
        assert!(require_year("year", "next").is_err());
    }

    #[test]
    fn rejects_blank_fields() {
        assert!(require_non_empty("club", "").is_err());
        assert!(require_non_empty("club", "   ").is_err());
        assert!(require_non_empty("club", "Ohio").is_ok());
    }
}
