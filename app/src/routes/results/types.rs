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
