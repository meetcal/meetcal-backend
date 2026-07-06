use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Serialize, Deserialize)]
pub struct MeetsParams {
    pub meet: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Meets {
    pub federation: String,
    pub end_date: String,
    pub name: String,
    pub start_date: String,
    pub time_zone: String,
    pub venue_city: String,
    pub venue_name: String,
    pub venue_state: String,
    pub venue_street: String,
    pub venue_zip: String,
    pub status: String,
    pub venue_map_pdf_url: Option<String>,
    pub venue_map_apple_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct MeetSchedule {
    pub date: String,
    pub meet: String,
    pub platform: String,
    pub session_id: f64,
    pub start_time: String,
    pub weigh_in_time: String,
    pub weight_class: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Athlete {
    pub member_id: String,
    pub adaptive: bool,
    pub age: f64,
    pub club: String,
    pub entry_total: f64,
    pub gender: String,
    pub meet: String,
    pub name: String,
    pub session_number: Option<f64>,
    pub session_platform: Option<String>,
    pub weight_class: String,
    pub wso: Option<String>,
}
