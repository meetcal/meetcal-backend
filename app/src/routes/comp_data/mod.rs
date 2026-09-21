pub mod get_adaptive_records;
pub mod get_intl_rankings;
pub mod get_national_ranking_by_year;
pub mod get_national_rankings;
pub mod get_qualifying_totals;
pub mod get_records;
pub mod get_standards;
pub mod get_wso_list;
pub mod get_wso_records;

use std::collections::HashMap;

/// Collapses ranking rows to one per athlete — their heaviest total — heaviest
/// first.
///
/// `lifting_results` stores one row per meet, so a national ranking has to pick
/// each athlete's best rather than list them once per competition. Both ranking
/// endpoints rank the same way; only the row shape differs.
fn best_total_per_athlete<T>(
    rows: Vec<T>,
    name: impl Fn(&T) -> &str,
    total: impl Fn(&T) -> f64,
) -> Vec<T> {
    let mut best: HashMap<String, T> = HashMap::new();
    for row in rows {
        match best.get(name(&row)) {
            // Ties keep the row seen first, as before.
            Some(existing) if total(existing) >= total(&row) => {}
            _ => {
                best.insert(name(&row).to_string(), row);
            }
        }
    }

    let mut ranked: Vec<T> = best.into_values().collect();
    ranked.sort_by(|left, right| total(right).total_cmp(&total(left)));
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Row {
        name: &'static str,
        total: f64,
    }

    fn rank(rows: Vec<Row>) -> Vec<(&'static str, f64)> {
        best_total_per_athlete(rows, |row| row.name, |row| row.total)
            .into_iter()
            .map(|row| (row.name, row.total))
            .collect()
    }

    #[test]
    fn keeps_each_athletes_heaviest_total_descending() {
        assert_eq!(
            rank(vec![
                Row {
                    name: "Ada",
                    total: 180.0
                },
                Row {
                    name: "Bo",
                    total: 240.0
                },
                Row {
                    name: "Ada",
                    total: 205.0
                },
                Row {
                    name: "Ada",
                    total: 195.0
                },
            ]),
            vec![("Bo", 240.0), ("Ada", 205.0)]
        );
    }

    #[test]
    fn empty_input_ranks_to_nothing() {
        assert_eq!(rank(Vec::new()), Vec::new());
    }
}
