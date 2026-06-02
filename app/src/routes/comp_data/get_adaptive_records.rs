use crate::common::query_convex::get_convex_response;
use crate::common::sort::sort_by_class;
use crate::routes::comp_data::types::LiftingResults;
use crate::{AppError, AppState};
use axum::Json;
use axum::extract::{Query, State};
use convex::Value;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdaptiveRecordsParams {
    pub exclude_federation: String,
    pub gender: String,
}

// #[derive(Debug, Deserialize, Serialize)]
// #[serde(rename_all = "camelCase")]
// pub struct AdaptiveRecords {
//     pub body_weight: f64,
//     pub age: String,
//     pub date: String,
//     pub snatch_best: f64,
//     pub cj_best: f64,
//     pub total: f64,
// }

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
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

/// /adaptive endpoint
///
/// curl 'http://localhost:3000/adaptive?excludeFederation=BWL&gender=Men' | jq .
///
/// Params:
/// Federations: USAW, USAMW, BWL
/// Gender: Men, Women
///
/// This endpoint takes an excluded federation (the db has some results for BWL) and gender and returns records for all adaptive athletes as a gender, no age brackets
///
/// [
///  {
///    "weightClass": "85",
///    "snatch": 40.0,
///    "cj": 50.0,
///    "total": 90.0
///  },
/// ]
pub async fn get_adaptive_records(
    State(state): State<AppState>,
    Query(params): Query<AdaptiveRecordsParams>,
) -> Result<Json<Vec<AdaptiveRecords>>, AppError> {
    let mut args = BTreeMap::new();
    args.insert(
        "excludeFederation".to_string(),
        Value::from(params.exclude_federation),
    );

    let gender = params.gender;

    let response: Vec<LiftingResults> =
        get_convex_response(&state.convex, "liftingResults:getAdaptive", args).await?;

    let mut records: HashMap<String, AdaptiveRecords> = HashMap::new();

    let filtered = response
        .iter()
        .filter(|g| extract_gender(g.age.as_str(), gender.as_str()))
        .filter(|y| extract_year(y.date.as_str()) >= 2026);

    for row in filtered {
        let class = extract_class(row.age.as_str()).unwrap();

        let current = records.get(&class).cloned().unwrap_or(AdaptiveRecords {
            weight_class: class.clone(),
            snatch: 0.0,
            cj: 0.0,
            total: 0.0,
        });

        records.insert(
            class.to_string(),
            AdaptiveRecords {
                weight_class: class.to_string(),
                snatch: current.snatch.max(row.snatch_best),
                cj: current.cj.max(row.cj_best),
                total: current.total.max(row.total),
            },
        );
    }

    let sorted = sort_by_class(records.into_values().collect(), |r| r.weight_class.as_str());

    Ok(Json(sorted))
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
