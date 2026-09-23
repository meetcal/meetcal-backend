use crate::common::{query::require_year, sort::sort_by_class};
use crate::routes::results::types::{LiftingResults, lifting_result_columns};
use crate::{AppError, AppState};
use axum::Json;
use axum::extract::{Query, State};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Debug, Deserialize, Serialize)]
pub struct AdaptiveRecordsParams {
    pub exclude_federation: String,
    pub gender: String,
    /// Four-digit year the record season starts on; defaults to
    /// [`ADAPTIVE_RECORDS_SEASON_START`].
    pub season: Option<String>,
}

/// USAW reset adaptive records with the 2026 weight classes, so results from
/// earlier seasons are not eligible. Overridable per request with `season=`.
pub const ADAPTIVE_RECORDS_SEASON_START: &str = "2026";

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct AdaptiveRecords {
    pub weight_class: String,
    pub snatch: f64,
    pub cj: f64,
    pub total: f64,
}

static MEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bmen\b").unwrap());
static WOMEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bwomen\b").unwrap());
static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d{4}\b").unwrap());
static WEIGHT_CLASS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(\d+\+?)kg").unwrap());

/// Only the season's rows for the requested gender leave Postgres. `$2` is
/// `YYYY-01-01`; `date` is text in ISO order so `>=` is the year filter. `$3`
/// is `true` for men. `age` is the scraped division label, so a gender is a
/// whole-word match (`\m` / `\M` are Postgres word boundaries, `~*` is
/// case-insensitive): men's rows match "men" and not "women", women's rows
/// match "women". [`extract_gender`] below is the same rule in Rust and
/// re-checks each row.
const ADAPTIVE_RESULTS_SQL: &str = concat!(
    r#"
        SELECT
            "#,
    lifting_result_columns!(),
    r#"
        FROM lifting_results
        WHERE adaptive = true
            AND (federation IS NULL OR federation <> $1)
            AND date >= $2
            AND CASE WHEN $3::boolean
                THEN COALESCE(age, '') ~* '\mmen\M' AND NOT COALESCE(age, '') ~* '\mwomen\M'
                ELSE COALESCE(age, '') ~* '\mwomen\M'
                END
        "#
);

/// /data/adaptive endpoint
///
/// curl 'https://api.meetcal.app/data/adaptive?exclude_federation=BWL&gender=Men' | jq .
///
/// Params:
/// Federations: USAW, USAMW, BWL
/// Gender: Men, Women
///
/// This endpoint takes an excluded federation (the db has some results for BWL) and gender and returns records for all adaptive athletes as a gender, no age brackets
///
/// Optional `season=YYYY` (default 2026, see [`ADAPTIVE_RECORDS_SEASON_START`]) is the first
/// year whose results count; anything else for `season` is `400`.
///
/// [
///  {
///    "weight_class": "85",
///    "snatch": 40.0,
///    "cj": 50.0,
///    "total": 90.0
///  },
/// ]
pub async fn get_adaptive_records(
    State(state): State<AppState>,
    Query(params): Query<AdaptiveRecordsParams>,
) -> Result<Json<Vec<AdaptiveRecords>>, AppError> {
    let season = params
        .season
        .as_deref()
        .unwrap_or(ADAPTIVE_RECORDS_SEASON_START);
    require_year("season", season)?;
    let men = params.gender.eq_ignore_ascii_case("men");
    let rows = sqlx::query_as::<_, LiftingResults>(ADAPTIVE_RESULTS_SQL)
        .bind(&params.exclude_federation)
        .bind(format!("{season}-01-01"))
        .bind(men)
        .fetch_all(&state.db)
        .await?;

    let season_start: u32 = season.parse().map_err(anyhow::Error::from)?;
    Ok(Json(best_by_weight_class(
        &rows,
        &params.gender,
        season_start,
    )))
}

/// Collapses adaptive result rows to one record per weight class, keeping the
/// heaviest snatch, clean and jerk, and total seen in each.
///
/// `age` is a scraped free-text combo of age group and weight class, so a row
/// can carry no `NNkg` token at all (`"Adaptive Men"`, or an empty `age`
/// column). Those rows have no class to file under and are skipped; reading the
/// class was previously an `unwrap`, which panicked the request.
fn best_by_weight_class(
    rows: &[LiftingResults],
    gender: &str,
    season_start: u32,
) -> Vec<AdaptiveRecords> {
    let mut records: HashMap<String, AdaptiveRecords> = HashMap::new();

    let filtered = rows
        .iter()
        .filter(|g| extract_gender(g.age.as_str(), gender))
        .filter(|y| extract_year(y.date.as_str()) >= season_start);

    for row in filtered {
        let Some(class) = extract_class(row.age.as_str()) else {
            continue;
        };

        let current = records.get(&class).cloned().unwrap_or(AdaptiveRecords {
            weight_class: class.clone(),
            snatch: 0.0,
            cj: 0.0,
            total: 0.0,
        });

        records.insert(
            class.clone(),
            AdaptiveRecords {
                weight_class: class,
                snatch: current.snatch.max(row.snatch_best),
                cj: current.cj.max(row.cj_best),
                total: current.total.max(row.total),
            },
        );
    }

    sort_by_class(records.into_values().collect(), |r| r.weight_class.as_str())
}

pub fn extract_gender(age: &str, gender: &str) -> bool {
    // Age in the db is a combo of age and weight class
    // Open Women's 86kg, Master's (40-44) Men's 95kg
    if gender.eq_ignore_ascii_case("men") {
        MEN.is_match(age) && !WOMEN.is_match(age)
    } else {
        WOMEN.is_match(age)
    }
}

pub fn extract_year(date: &str) -> u32 {
    // get year from date string
    YEAR.find(date)
        .and_then(|matched| matched.as_str().parse().ok())
        .unwrap_or(0)
}

pub fn extract_class(age: &str) -> Option<String> {
    // weight class is last portion of age db column
    // get numbers before kg, including + if there
    WEIGHT_CLASS
        .captures_iter(age)
        .filter_map(|cap| {
            let matched = cap.get(1)?;
            if is_inside_parens(age, matched.start()) {
                return None;
            }
            Some(matched.as_str().to_string())
        })
        .next()
}

fn is_inside_parens(text: &str, index: usize) -> bool {
    let before = &text[..index];
    let Some(open) = before.rfind('(') else {
        return false;
    };

    !before[open..].contains(')')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(age: &str, date: &str, snatch: f64, cj: f64, total: f64) -> LiftingResults {
        LiftingResults {
            id: 1,
            event_id: "event_2026".to_string(),
            federation: "USAW".to_string(),
            meet: "2026 Adaptive Nationals".to_string(),
            date: date.to_string(),
            name: "Adaptive Test Athlete".to_string(),
            age: age.to_string(),
            body_weight: 84.5,
            snatch1: 0.0,
            snatch2: 0.0,
            snatch3: 0.0,
            snatch_best: snatch,
            cj1: 0.0,
            cj2: 0.0,
            cj3: 0.0,
            cj_best: cj,
            total,
            adaptive: true,
        }
    }

    #[test]
    fn rows_without_a_weight_class_are_skipped_not_panicked() {
        // `age` is scraped free text; these three carry no `NNkg` token, and
        // reading one used to panic the request.
        let rows = vec![
            row("Adaptive Men", "2026-02-01", 60.0, 70.0, 130.0),
            row("Men", "2026-02-01", 61.0, 71.0, 132.0),
            // A class that only appears parenthesized is not a class either.
            row("Adaptive Men (85kg group)", "2026-02-01", 62.0, 72.0, 134.0),
            row("Adaptive Men 85kg", "2026-02-01", 40.0, 50.0, 90.0),
        ];

        assert_eq!(
            best_by_weight_class(&rows, "Men", 2026),
            vec![AdaptiveRecords {
                weight_class: "85".to_string(),
                snatch: 40.0,
                cj: 50.0,
                total: 90.0,
            }]
        );
    }

    #[test]
    fn keeps_the_heaviest_lift_per_class_in_class_order() {
        let rows = vec![
            row("Adaptive Men 85kg", "2026-02-01", 40.0, 50.0, 90.0),
            row("Adaptive Men 85kg", "2026-03-01", 45.0, 45.0, 88.0),
            row("Adaptive Men 110+kg", "2026-02-01", 80.0, 90.0, 170.0),
            // Before the 2026 cutoff, and the wrong gender: both excluded.
            row("Adaptive Men 85kg", "2025-02-01", 999.0, 999.0, 999.0),
            row("Adaptive Women 85kg", "2026-02-01", 999.0, 999.0, 999.0),
        ];

        assert_eq!(
            best_by_weight_class(&rows, "Men", 2026),
            vec![
                AdaptiveRecords {
                    weight_class: "85".to_string(),
                    snatch: 45.0,
                    cj: 50.0,
                    total: 90.0,
                },
                AdaptiveRecords {
                    weight_class: "110+".to_string(),
                    snatch: 80.0,
                    cj: 90.0,
                    total: 170.0,
                },
            ]
        );
    }
}
