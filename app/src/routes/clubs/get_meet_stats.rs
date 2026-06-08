use crate::{AppError, AppState};
use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

#[derive(Debug, Deserialize, Serialize)]
pub struct ClubMeetStatsParams {
    pub club: String,
    pub meet: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetStats {
    pub total_athletes: i64,
    pub gold_medals: i64,
    pub silver_medals: i64,
    pub bronze_medals: i64,
    pub total_prs: i64,
    pub perfect6for6: i64,
    pub total_weight_lifted: f64,
    pub athlete_results: Vec<AthleteMeetResult>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AthleteMeetResult {
    pub name: String,
    pub snatch_best: f64,
    pub cj_best: f64,
    pub total: f64,
    pub body_weight: f64,
    pub medal: Option<String>,
    pub is_pr: bool,
    pub perfect_lifts: bool,
}

#[derive(Debug, FromRow)]
struct ClubResultRow {
    name: String,
    body_weight: f64,
    snatch1: f64,
    snatch2: f64,
    snatch3: f64,
    snatch_best: f64,
    cj1: f64,
    cj2: f64,
    cj3: f64,
    cj_best: f64,
    total: f64,
    placing: i64,
    previous_best_total: Option<f64>,
}

/// /clubs/meet-stats endpoint
///
/// curl 'https://api.meetcal.app/clubs/meet-stats?meet=2026%20Ohio%20WSO%20Championships&club=Columbus%20Weightlifting' | jq .
///
/// This endpoint takes meet and club name and returns a full report of how the club did at the meet
///
/// TODO: Reshape for the RN app as a screen-ready club report. The backend should calculate PRs,
/// 6-for-6, total lifted, and snatch/CJ/total medals by weight class so the app only renders.
///
/// {
///   "totalAthletes": 1,
///   "goldMedals": 0,
///   "silverMedals": 0,
///   "bronzeMedals": 0,
///   "totalPRs": 0,
///   "perfect6for6": 0,
///   "totalWeightLifted": 0.0,
///   "athleteResults": []
/// }
pub async fn get_meet_stats(
    State(state): State<AppState>,
    Query(params): Query<ClubMeetStatsParams>,
) -> Result<Json<MeetStats>, AppError> {
    let total_athletes: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)::BIGINT
        FROM athletes
        WHERE club = $1
            AND meet = $2
        "#,
    )
    .bind(&params.club)
    .bind(&params.meet)
    .fetch_one(&state.db)
    .await?;

    let rows: Vec<ClubResultRow> = sqlx::query_as(
        r#"
        WITH club_athletes AS (
            SELECT DISTINCT name
            FROM athletes
            WHERE club = $1
                AND meet = $2
        ),
        meet_results AS (
            SELECT
                lr.*,
                RANK() OVER (
                    PARTITION BY COALESCE(lr.age, '')
                    ORDER BY COALESCE(lr.total, 0) DESC
                ) AS placing
            FROM lifting_results lr
            WHERE lr.meet = $2
        )
        SELECT
            mr.name,
            COALESCE(mr.body_weight, 0) AS body_weight,
            COALESCE(mr.snatch1, 0) AS snatch1,
            COALESCE(mr.snatch2, 0) AS snatch2,
            COALESCE(mr.snatch3, 0) AS snatch3,
            COALESCE(mr.snatch_best, 0) AS snatch_best,
            COALESCE(mr.cj1, 0) AS cj1,
            COALESCE(mr.cj2, 0) AS cj2,
            COALESCE(mr.cj3, 0) AS cj3,
            COALESCE(mr.cj_best, 0) AS cj_best,
            COALESCE(mr.total, 0) AS total,
            mr.placing,
            (
                SELECT MAX(previous.total)
                FROM lifting_results previous
                WHERE previous.name = mr.name
                    AND previous.date < mr.date
            ) AS previous_best_total
        FROM meet_results mr
        INNER JOIN club_athletes ca ON ca.name = mr.name
        ORDER BY mr.name
        "#,
    )
    .bind(&params.club)
    .bind(&params.meet)
    .fetch_all(&state.db)
    .await?;

    let gold_medals = rows.iter().filter(|row| row.placing == 1).count() as i64;
    let silver_medals = rows.iter().filter(|row| row.placing == 2).count() as i64;
    let bronze_medals = rows.iter().filter(|row| row.placing == 3).count() as i64;

    let total_prs = rows
        .iter()
        .filter(|row| {
            row.previous_best_total
                .is_some_and(|previous| row.total > previous)
        })
        .count() as i64;

    let perfect6for6 = rows
        .iter()
        .filter(|row| {
            [
                row.snatch1,
                row.snatch2,
                row.snatch3,
                row.cj1,
                row.cj2,
                row.cj3,
            ]
            .iter()
            .all(|attempt| *attempt > 0.0)
        })
        .count() as i64;

    let total_weight_lifted = rows.iter().map(|row| row.total).sum();

    let athlete_results = rows
        .into_iter()
        .map(|row| AthleteMeetResult {
            name: row.name,
            snatch_best: row.snatch_best,
            cj_best: row.cj_best,
            total: row.total,
            body_weight: row.body_weight,
            medal: match row.placing {
                1 => Some("gold".to_string()),
                2 => Some("silver".to_string()),
                3 => Some("bronze".to_string()),
                _ => None,
            },
            is_pr: row
                .previous_best_total
                .is_some_and(|previous| row.total > previous),
            perfect_lifts: [
                row.snatch1,
                row.snatch2,
                row.snatch3,
                row.cj1,
                row.cj2,
                row.cj3,
            ]
            .iter()
            .all(|attempt| *attempt > 0.0),
        })
        .collect();

    Ok(Json(MeetStats {
        total_athletes: total_athletes.0,
        gold_medals,
        silver_medals,
        bronze_medals,
        total_prs,
        perfect6for6,
        total_weight_lifted,
        athlete_results,
    }))
}
