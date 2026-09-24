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
/// first, athletes with equal totals in name order.
///
/// `lifting_results` stores one row per meet, so a national ranking has to pick
/// each athlete's best rather than list them once per competition. Both ranking
/// endpoints rank the same way; only the row shape differs.
///
/// The name tie-break makes the order a function of the data alone. Without it
/// equal totals came out in `HashMap` iteration order, which differs per
/// request, so the body (and its strong `ETag`) would change between two
/// identical requests and a revalidation could never be a `304`.
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
    ranked.sort_by(|left, right| {
        total(right)
            .total_cmp(&total(left))
            .then_with(|| name(left).cmp(name(right)))
    });
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
    fn equal_totals_rank_in_name_order_whatever_the_input_order() {
        let tied = |names: [&'static str; 3]| {
            rank(
                names
                    .into_iter()
                    .map(|name| Row { name, total: 200.0 })
                    .collect(),
            )
        };
        let expected = vec![("Ada", 200.0), ("Bo", 200.0), ("Cy", 200.0)];
        assert_eq!(tied(["Cy", "Ada", "Bo"]), expected);
        assert_eq!(tied(["Bo", "Cy", "Ada"]), expected);
    }

    #[test]
    fn empty_input_ranks_to_nothing() {
        assert_eq!(rank(Vec::new()), Vec::new());
    }
}
