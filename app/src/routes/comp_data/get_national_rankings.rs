use crate::common::query_convex::get_convex_response;
use crate::{AppError, AppState};
use axum::Json;
use axum::extract::{Query, State};
use convex::Value;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NatRankingsParams {
    pub age_category: String,
    pub federation: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NatRankings {
    pub name: String,
    pub total: f64,
}

/// /nat-rankings endpoint
///
/// curl 'http://localhost:3000/nat-rankings?ageCategory=Open%20Men%27s%2060kg&federation=USAW' | jq .
///
/// Age Categories:
///  "Men's 11 Under Age Group 32kg",
///  "Women's 11 Under Age Group 30kg",
///  "Men's 13 Under Age Group 32kg",
///  "Women's 13 Under Age Group 30kg",
///  "Men's 14-15 Age Group 48kg",
///  "Women's 14-15 Age Group 40kg",
///  "Men's 16-17 Age Group 56kg",
///  "Junior Men's 110+kg",
///  "Junior Women's 48kg",
///  "Open Men's 110+kg",
///  "Open Women's 48kg",
///  "Men's Masters (35-39) 110+kg",
///  "Women's Masters (35-39) 53kg",
///  "Men's Masters (40-44) 110+kg",
///  "Men's Masters (45-49) 110+kg",
///  "Men's Masters (50-54) 110+kg",
///  ...
///  "Men's Masters (80-84) 94kg",
/// Federations: USAW, USAMW
///
/// This endpoint takes federation and age category and returns national rankings for a weight_class
///
/// [
///  {
///    "name": "gabe chhum",
///    "total": 281.0
///  },
///  {
///    "name": "Kaiden Mima",
///    "total": 247.0
///  },
/// ]
pub async fn get_national_rankings(
    State(state): State<AppState>,
    Query(params): Query<NatRankingsParams>,
) -> Result<Json<Vec<NatRankings>>, AppError> {
    let mut args = BTreeMap::new();
    args.insert("ageCategory".to_string(), Value::from(params.age_category));
    args.insert("federation".to_string(), Value::from(params.federation));

    let response: Vec<NatRankings> =
        get_convex_response(&state.convex, "liftingResults:getNatRankings", args).await?;

    let mut rows_hash: HashMap<String, NatRankings> = HashMap::new();

    // if not in map insert, else if name is in map, check total, compare, keep highest
    for row in response {
        if rows_hash.contains_key(&row.name) {
            // this is the val associated with the key
            let entry = rows_hash.get_mut(&row.name).unwrap();
            if row.total > entry.total {
                *entry = row;
            }
        } else {
            rows_hash.insert(row.name.clone(), row);
        }
    }

    // pull out just the vals from the HashMap
    let mut row_array: Vec<NatRankings> = rows_hash.into_values().collect();
    row_array.sort_by(|a, b| b.total.total_cmp(&a.total));

    Ok(Json(row_array))
}
