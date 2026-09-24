use crate::common::names::{normalize_name, normalized_name_sql};
use crate::{AppError, AppState, common::client::ClientVersion};
use axum::extract::State;
use axum::{Json, extract::Query};
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionsAthletesParams {
    pub meet: String,
    pub session_number: Option<f64>,
    pub platform: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, FromRow)]
pub struct SessionsAthletes {
    pub member_id: String,
    pub name: String,
    pub age: f64,
    pub club: String,
    pub wso: Option<String>,
    pub gender: String,
    pub weight_class: String,
    pub entry_total: f64,
    pub adaptive: bool,
    pub session_number: Option<f64>,
    pub session_platform: Option<String>,
    pub date: Option<String>,
    pub start_time: Option<String>,
    pub weigh_in_time: Option<String>,
}

/// One projection for all four filter combinations, so the column list and the
/// schedule join condition exist once.
///
/// `$join` is `LEFT JOIN` only for the unfiltered variant, which must still
/// list athletes whose session has no schedule row yet; the filtered variants
/// match on a session/platform that by definition has one. `$filters` is
/// appended to the `WHERE`. Both are literals written here (or `concat!`s of
/// them), never caller input, and the expansion is a string literal so the
/// query stays `&'static str`.
macro_rules! sessions_for_athletes_sql {
    ($join:literal, $filters:expr) => {
        concat!(
            r#"
        SELECT
            a.member_id,
            a.name,
            a.age,
            a.club,
            a.wso,
            a.gender,
            a.weight_class,
            a.entry_total,
            a.adaptive,
            a.session_number,
            a.session_platform,
            s.date,
            s.start_time,
            s.weigh_in_time
        FROM athletes a
        "#,
            $join,
            r#" session_schedule s
            ON s.meet = a.meet
            AND s.session_id = a.session_number
            AND s.platform = a.session_platform
        WHERE a.meet = $1
            "#,
            $filters,
            r#"
        "#
        )
    };
}

/// `session_platform` is free text. Ingest canonicalises it now ("red " ->
/// "Red", the app sends the same canonical form), but rows written before
/// that, or by a source with its own casing, may still hold "RED" or "red ".
/// The filter therefore compares by the case- and whitespace-insensitive rule
/// names use, with the parameter normalized the same way (`normalize_name`),
/// so the app's canonical value matches whatever spelling is stored. The
/// `meet` predicate keeps the lookup on `idx_athletes_meet*`; the platform
/// test then runs over one meet's roster.
const BY_SESSION_AND_PLATFORM_SQL: &str = sessions_for_athletes_sql!(
    "JOIN",
    concat!(
        "AND a.session_number = $2\n            AND ",
        normalized_name_sql!("a.session_platform"),
        " = $3"
    )
);
const BY_SESSION_SQL: &str = sessions_for_athletes_sql!("JOIN", "AND a.session_number = $2");
const BY_PLATFORM_SQL: &str = sessions_for_athletes_sql!(
    "JOIN",
    concat!("AND ", normalized_name_sql!("a.session_platform"), " = $2")
);
const ALL_SESSIONS_SQL: &str = sessions_for_athletes_sql!("LEFT JOIN", "");

/// /meets/athletes-sessions endpoint
///
/// curl 'https://api.meetcal.app/meets/athletes-sessions?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness' | jq .
///
/// This endpoint takes meet name and returns athletes and their session rows.
/// Optional session_number and platform filters return one session/platform.
/// `platform` matches case- and whitespace-insensitively (`red`, `RED `, `Red`
/// are one platform); the rows carry the stored spelling.
///
/// A blank `meet` is `400` for a 6.2.0+ client and `200 []` for a legacy one.
///
/// `start_time` and `weigh_in_time` are the `session_schedule` free text as ingested
/// (`"08:00:00"`, `"10:00"`, `"10:00 AM"` have all been seen); the app parses
/// `h:mm[:ss][ AM/PM]`. They are `null` when the athlete's session has no schedule row.
///
/// [
///     {
///       "member_id": "12345",
///       "name": "Jane Doe",
///       "age": 28,
///       "club": "ABC Weightlifting",
///       "wso": "Mountain South",
///       "gender": "Women",
///       "weight_class": "71kg",
///       "entry_total": 180,
///       "adaptive": false,
///       "session_number": 3,
///       "session_platform": "Red",
///       "date": "2025-05-31",
///       "start_time": "10:00",
///       "weigh_in_time": "08:00",
///     }
/// ]
pub async fn get_sessions_for_athletes(
    State(state): State<AppState>,
    client: ClientVersion,
    Query(params): Query<SessionsAthletesParams>,
) -> Result<Json<Vec<SessionsAthletes>>, AppError> {
    client.require_non_empty("meet", &params.meet)?;
    let rows: Vec<SessionsAthletes> = match (params.session_number, params.platform) {
        (Some(session_number), Some(platform)) => {
            sqlx::query_as(BY_SESSION_AND_PLATFORM_SQL)
                .bind(&params.meet)
                .bind(session_number)
                .bind(normalize_name(&platform))
                .fetch_all(&state.db)
                .await?
        }
        (Some(session_number), None) => {
            sqlx::query_as(BY_SESSION_SQL)
                .bind(&params.meet)
                .bind(session_number)
                .fetch_all(&state.db)
                .await?
        }
        (None, Some(platform)) => {
            sqlx::query_as(BY_PLATFORM_SQL)
                .bind(&params.meet)
                .bind(normalize_name(&platform))
                .fetch_all(&state.db)
                .await?
        }
        (None, None) => {
            sqlx::query_as(ALL_SESSIONS_SQL)
                .bind(&params.meet)
                .fetch_all(&state.db)
                .await?
        }
    };

    Ok(Json(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_filters_compare_case_and_whitespace_insensitively() {
        let rule = "lower(btrim(regexp_replace(a.session_platform, '\\s+', ' ', 'g')))";
        assert!(BY_PLATFORM_SQL.contains(&format!("AND {rule} = $2")));
        assert!(BY_SESSION_AND_PLATFORM_SQL.contains(&format!("AND {rule} = $3")));
        assert!(BY_SESSION_AND_PLATFORM_SQL.contains("AND a.session_number = $2"));
        // The schedule join itself stays exact: both sides are written by ingest.
        assert!(BY_PLATFORM_SQL.contains("AND s.platform = a.session_platform"));
    }
}
