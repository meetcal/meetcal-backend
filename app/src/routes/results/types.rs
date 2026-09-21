use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Deserialize, Serialize, Clone, FromRow)]
pub struct LiftingResults {
    pub federation: String,
    pub meet: String,
    pub date: String,
    pub name: String,
    pub age: String,
    pub body_weight: f64,
    pub snatch1: f64,
    pub snatch2: f64,
    pub snatch3: f64,
    pub snatch_best: f64,
    pub cj1: f64,
    pub cj2: f64,
    pub cj3: f64,
    pub cj_best: f64,
    pub total: f64,
    pub adaptive: bool,
}

/// The `lifting_results` projection that fills [`LiftingResults`]. Every
/// endpoint that returns result rows selects exactly these columns, so adding a
/// field to the struct means editing one list, not eight queries.
macro_rules! lifting_result_columns {
    () => {
        "COALESCE(federation, '') AS federation,
            meet,
            date,
            name,
            COALESCE(age, '') AS age,
            COALESCE(body_weight, 0) AS body_weight,
            COALESCE(snatch1, 0) AS snatch1,
            COALESCE(snatch2, 0) AS snatch2,
            COALESCE(snatch3, 0) AS snatch3,
            COALESCE(snatch_best, 0) AS snatch_best,
            COALESCE(cj1, 0) AS cj1,
            COALESCE(cj2, 0) AS cj2,
            COALESCE(cj3, 0) AS cj3,
            COALESCE(cj_best, 0) AS cj_best,
            COALESCE(total, 0) AS total,
            adaptive"
    };
}
pub(crate) use lifting_result_columns;

/// Aggregate projection for "best lifts in a window": the heaviest successful
/// snatch, clean and jerk, and total over the rows a query selects. Shared by
/// `/lifting-results/year`, `/lifting-results/bests`, and `/meets/package`.
macro_rules! best_lifts_columns {
    () => {
        "COALESCE(MAX(GREATEST(
                COALESCE(snatch_best, 0),
                COALESCE(snatch1, 0),
                COALESCE(snatch2, 0),
                COALESCE(snatch3, 0)
            )), 0) AS best_snatch,
            COALESCE(MAX(GREATEST(
                COALESCE(cj_best, 0),
                COALESCE(cj1, 0),
                COALESCE(cj2, 0),
                COALESCE(cj3, 0)
            )), 0) AS best_cj,
            COALESCE(MAX(COALESCE(total, 0)), 0) AS best_total"
    };
}
pub(crate) use best_lifts_columns;

#[cfg(test)]
mod tests {
    /// Splits a SQL select list on the commas that separate columns, ignoring
    /// the ones nested inside `COALESCE(...)` / `GREATEST(...)`.
    fn output_names(select_list: &str) -> Vec<String> {
        let mut depth = 0usize;
        let mut current = String::new();
        let mut items = Vec::new();
        for character in select_list.chars() {
            match character {
                '(' => {
                    depth += 1;
                    current.push(character);
                }
                ')' => {
                    depth -= 1;
                    current.push(character);
                }
                ',' if depth == 0 => items.push(std::mem::take(&mut current)),
                _ => current.push(character),
            }
        }
        items.push(current);

        items
            .iter()
            .map(|item| {
                let item = item.split_whitespace().collect::<Vec<_>>().join(" ");
                match item.rsplit_once(" AS ") {
                    Some((_, alias)) => alias.to_string(),
                    None => item,
                }
            })
            .collect()
    }

    #[test]
    fn projection_matches_the_lifting_results_struct() {
        // A column added to `LiftingResults` without adding it here would make
        // every result endpoint fail to decode at runtime; sqlx cannot catch it
        // because the queries are not compile-time checked.
        assert_eq!(
            output_names(lifting_result_columns!()),
            vec![
                "federation",
                "meet",
                "date",
                "name",
                "age",
                "body_weight",
                "snatch1",
                "snatch2",
                "snatch3",
                "snatch_best",
                "cj1",
                "cj2",
                "cj3",
                "cj_best",
                "total",
                "adaptive",
            ]
        );
    }

    #[test]
    fn best_lifts_projection_names_the_three_bests() {
        assert_eq!(
            output_names(best_lifts_columns!()),
            vec!["best_snatch", "best_cj", "best_total"]
        );
    }
}
