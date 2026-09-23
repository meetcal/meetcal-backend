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

/// SQL expression that normalizes a name column to match [`normalize_name`].
/// Compare it against a parameter already normalized with [`normalize_name`].
///
/// This is the only spelling of the rule in the crate; the `normalized_name`
/// indexes in `app/migrations/` are built on the same expression. `concat!` it
/// into a query instead of retyping it — the expansion is a string literal, so
/// queries stay `&'static str` and keep sqlx's no-dynamic-SQL guarantee.
///
/// `normalized_name_sql!()` normalizes the bare `name` column;
/// `normalized_name_sql!("lr.name")` qualifies it for self-joins. The column is
/// a literal written by this crate, never caller input.
macro_rules! normalized_name_sql {
    () => {
        normalized_name_sql!("name")
    };
    ($column:literal) => {
        concat!(
            "lower(btrim(regexp_replace(",
            $column,
            ", '\\s+', ' ', 'g')))"
        )
    };
}
pub(crate) use normalized_name_sql;

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

    #[test]
    fn sql_expression_matches_the_rust_normalizer() {
        // The qualified form must stay the same rule as the unqualified one, or
        // a self-join would match on different keys.
        assert_eq!(
            normalized_name_sql!(),
            "lower(btrim(regexp_replace(name, '\\s+', ' ', 'g')))"
        );
        assert_eq!(
            normalized_name_sql!("lr.name"),
            "lower(btrim(regexp_replace(lr.name, '\\s+', ' ', 'g')))"
        );
    }
}
