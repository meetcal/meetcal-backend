/// Normalizes an athlete name for case- and whitespace-insensitive matching.
///
/// Collapses runs of internal whitespace to a single space, trims, and
/// lowercases.
///
/// Example: `"Anna Mcelderry"` and `"Anna  McElderry "` both normalize to
/// `"anna mcelderry"`.
pub fn normalize_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// SQL expression that normalizes a `name` column to match [`normalize_name`].
/// Compare it against a parameter already normalized with [`normalize_name`].
pub const NORMALIZED_NAME_SQL: &str = "lower(btrim(regexp_replace(name, '\\s+', ' ', 'g')))";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_case_and_whitespace() {
        assert_eq!(normalize_name("Anna McElderry"), "anna mcelderry");
        assert_eq!(normalize_name("Anna Mcelderry"), "anna mcelderry");
        assert_eq!(normalize_name("  Anna   McElderry  "), "anna mcelderry");
        assert_eq!(normalize_name("ANNA MCELDERRY"), "anna mcelderry");
    }
}
