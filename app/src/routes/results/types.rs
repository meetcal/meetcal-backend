use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConvexLiftingResults {
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

impl From<ConvexLiftingResults> for LiftingResults {
    fn from(row: ConvexLiftingResults) -> Self {
        Self {
            federation: row.federation,
            meet: row.meet,
            date: row.date,
            name: row.name,
            age: row.age,
            body_weight: row.body_weight,
            snatch1: row.snatch1,
            snatch2: row.snatch2,
            snatch3: row.snatch3,
            snatch_best: row.snatch_best,
            cj1: row.cj1,
            cj2: row.cj2,
            cj3: row.cj3,
            cj_best: row.cj_best,
            total: row.total,
            adaptive: row.adaptive,
        }
    }
}
