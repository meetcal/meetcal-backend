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
    fn rejects_blank_fields() {
        assert!(require_non_empty("club", "").is_err());
        assert!(require_non_empty("club", "   ").is_err());
        assert!(require_non_empty("club", "Ohio").is_ok());
    }
}
