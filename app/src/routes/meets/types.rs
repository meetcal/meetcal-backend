use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meets {
    pub end_date: String,
    pub name: String,
    pub start_date: String,
    pub time_zone: String,
    pub venue_city: String,
    pub venue_name: String,
    pub venue_state: String,
    pub venue_street: String,
    pub venue_zip: String,
}
