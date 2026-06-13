use crate::{AppError, AppState};
use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Deserialize, Serialize)]
pub struct MeetPackageParams {
    pub meet: String,
    pub history_cutoff_date: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MeetPackage {
    pub meet: PackageMeet,
    pub schedule: Vec<PackageScheduleDay>,
    pub athletes: Vec<PackageAthlete>,
    pub meet_results: Vec<PackageLiftingResult>,
    pub year_bests_by_name: BTreeMap<String, YearBests>,
    pub recent_results_by_name: BTreeMap<String, Vec<PackageLiftingResult>>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct PackageMeet {
    pub id: String,
    pub name: String,
    pub federation: String,
    pub status: String,
    pub start_date: String,
    pub end_date: String,
    pub time_zone: String,
    pub venue_name: String,
    pub venue_street: String,
    pub venue_city: String,
    pub venue_state: String,
    pub venue_zip: String,
}

#[derive(Debug, Serialize)]
pub struct PackageScheduleDay {
    pub date: String,
    pub sessions: Vec<PackageScheduleSession>,
}

#[derive(Debug, Serialize)]
pub struct PackageScheduleSession {
    pub session_id: f64,
    pub start_time: String,
    pub weigh_in_time: String,
    pub platforms: Vec<PackageSchedulePlatform>,
}

#[derive(Debug, Serialize)]
pub struct PackageSchedulePlatform {
    pub platform: String,
    pub weight_class: String,
}

#[derive(Debug, Serialize)]
pub struct PackageAthlete {
    pub member_id: String,
    pub name: String,
    pub age: f64,
    pub club: String,
    pub wso: Option<String>,
    pub gender: String,
    pub weight_class: String,
    pub entry_total: f64,
    pub adaptive: bool,
    pub session: Option<PackageAthleteSession>,
}

#[derive(Debug, Serialize)]
pub struct PackageAthleteSession {
    pub session_number: f64,
    pub session_platform: String,
    pub date: Option<String>,
    pub start_time: Option<String>,
    pub weigh_in_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct PackageLiftingResult {
    pub id: i64,
    pub event_id: String,
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

#[derive(Debug, Clone, Serialize)]
pub struct YearBests {
    pub best_snatch: f64,
    pub best_cj: f64,
    pub best_total: f64,
}

#[derive(Debug, FromRow)]
struct ScheduleRow {
    date: String,
    session_id: f64,
    start_time: String,
    weigh_in_time: String,
    platform: String,
    weight_class: String,
}

#[derive(Debug, FromRow)]
struct AthletePackageRow {
    member_id: String,
    name: String,
    age: f64,
    club: String,
    wso: Option<String>,
    gender: String,
    weight_class: String,
    entry_total: f64,
    adaptive: bool,
    session_number: Option<f64>,
    session_platform: Option<String>,
    date: Option<String>,
    start_time: Option<String>,
    weigh_in_time: Option<String>,
}

/// /meets/package endpoint
///
/// curl 'https://api.meetcal.app/meets/package?meet=2026%20Ohio%20WSO%20Championships&history_cutoff_date=2024-06-13' | jq .
///
/// This endpoint returns a selected meet package for app screens that share the same meet data:
/// schedule, start list, schedule details, offline download, attempt estimator, and cached bests.
/// If history_cutoff_date is omitted, recent_results_by_name and year_bests_by_name are empty.
///
/// {
///   "meet": {
///     "id": "meet_ohio_2026",
///     "name": "2026 Ohio WSO Championships",
///     "federation": "USAW",
///     "status": "completed",
///     "start_date": "2026-05-01",
///     "end_date": "2026-05-03",
///     "time_zone": "America/New_York",
///     "venue_name": "Ohio Expo Center",
///     "venue_street": "717 E 17th Ave",
///     "venue_city": "Columbus",
///     "venue_state": "OH",
///     "venue_zip": "43211"
///   },
///   "schedule": [],
///   "athletes": [],
///   "meet_results": [],
///   "year_bests_by_name": {},
///   "recent_results_by_name": {}
/// }
pub async fn get_meet_package(
    State(state): State<AppState>,
    Query(params): Query<MeetPackageParams>,
) -> Result<Json<MeetPackage>, AppError> {
    let meet = sqlx::query_as::<_, PackageMeet>(
        r#"
        SELECT
            convex_id AS id,
            name,
            federation,
            status,
            start_date::text AS start_date,
            end_date::text AS end_date,
            time_zone,
            venue_name,
            venue_street,
            venue_city,
            venue_state,
            venue_zip
        FROM meets
        WHERE name = $1
        "#,
    )
    .bind(&params.meet)
    .fetch_one(&state.db)
    .await?;

    let schedule_rows = sqlx::query_as::<_, ScheduleRow>(
        r#"
        SELECT date, session_id, start_time, weigh_in_time, platform, weight_class
        FROM session_schedule
        WHERE meet = $1
        ORDER BY date, session_id, platform
        "#,
    )
    .bind(&params.meet)
    .fetch_all(&state.db)
    .await?;

    let athletes = sqlx::query_as::<_, AthletePackageRow>(
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
        LEFT JOIN session_schedule s
            ON s.meet = a.meet
            AND s.session_id = a.session_number
            AND s.platform = a.session_platform
        WHERE a.meet = $1
        ORDER BY a.name
        "#,
    )
    .bind(&params.meet)
    .fetch_all(&state.db)
    .await?;

    let meet_results = sqlx::query_as::<_, PackageLiftingResult>(
        r#"
        SELECT
            id,
            event_id,
            COALESCE(federation, '') AS federation,
            meet,
            date,
            name,
            COALESCE(age, '') AS age,
            COALESCE(body_weight, 0) AS body_weight,
            COALESCE(snatch1, 0) AS snatch1,
            COALESCE(snatch2, 0) AS snatch2,
            COALESCE(snatch3, 0) AS snatch3,
            COALESCE(snatch_best, 0) AS snatch_best,
            COALESCE(cj1, 0) AS cj1,
            COALESCE(cj2, 0) AS cj2,
            COALESCE(cj3, 0) AS cj3,
            COALESCE(cj_best, 0) AS cj_best,
            COALESCE(total, 0) AS total,
            adaptive
        FROM lifting_results
        WHERE meet = $1
        ORDER BY name, date DESC
        "#,
    )
    .bind(&params.meet)
    .fetch_all(&state.db)
    .await?;

    let athlete_names: Vec<String> = athletes
        .iter()
        .map(|athlete| athlete.name.clone())
        .collect();

    let (recent_results_by_name, year_bests_by_name) =
        if let Some(cutoff_date) = params.history_cutoff_date.as_ref() {
            let history_rows = if athlete_names.is_empty() {
                Vec::new()
            } else {
                sqlx::query_as::<_, PackageLiftingResult>(
                    r#"
                    SELECT
                        id,
                        event_id,
                        COALESCE(federation, '') AS federation,
                        meet,
                        date,
                        name,
                        COALESCE(age, '') AS age,
                        COALESCE(body_weight, 0) AS body_weight,
                        COALESCE(snatch1, 0) AS snatch1,
                        COALESCE(snatch2, 0) AS snatch2,
                        COALESCE(snatch3, 0) AS snatch3,
                        COALESCE(snatch_best, 0) AS snatch_best,
                        COALESCE(cj1, 0) AS cj1,
                        COALESCE(cj2, 0) AS cj2,
                        COALESCE(cj3, 0) AS cj3,
                        COALESCE(cj_best, 0) AS cj_best,
                        COALESCE(total, 0) AS total,
                        adaptive
                    FROM lifting_results
                    WHERE name = ANY($1::text[])
                        AND date >= $2
                    ORDER BY name, date DESC
                    "#,
                )
                .bind(&athlete_names)
                .bind(cutoff_date)
                .fetch_all(&state.db)
                .await?
            };

            build_history_maps(&athlete_names, history_rows)
        } else {
            (BTreeMap::new(), BTreeMap::new())
        };

    Ok(Json(MeetPackage {
        meet,
        schedule: build_schedule(schedule_rows),
        athletes: athletes.into_iter().map(PackageAthlete::from).collect(),
        meet_results,
        year_bests_by_name,
        recent_results_by_name,
    }))
}

fn build_schedule(rows: Vec<ScheduleRow>) -> Vec<PackageScheduleDay> {
    let mut days: Vec<PackageScheduleDay> = Vec::new();

    for row in rows {
        let day_index = days.iter().position(|day| day.date == row.date);
        let day_index = match day_index {
            Some(index) => index,
            None => {
                days.push(PackageScheduleDay {
                    date: row.date.clone(),
                    sessions: Vec::new(),
                });
                days.len() - 1
            }
        };

        let sessions = &mut days[day_index].sessions;
        let session_index = sessions
            .iter()
            .position(|session| session.session_id == row.session_id);
        let session_index = match session_index {
            Some(index) => index,
            None => {
                sessions.push(PackageScheduleSession {
                    session_id: row.session_id,
                    start_time: row.start_time.clone(),
                    weigh_in_time: row.weigh_in_time.clone(),
                    platforms: Vec::new(),
                });
                sessions.len() - 1
            }
        };

        sessions[session_index]
            .platforms
            .push(PackageSchedulePlatform {
                platform: row.platform,
                weight_class: row.weight_class,
            });
    }

    days
}

fn build_history_maps(
    athlete_names: &[String],
    rows: Vec<PackageLiftingResult>,
) -> (
    BTreeMap<String, Vec<PackageLiftingResult>>,
    BTreeMap<String, YearBests>,
) {
    let mut recent_results_by_name: BTreeMap<String, Vec<PackageLiftingResult>> = athlete_names
        .iter()
        .map(|name| (name.clone(), Vec::new()))
        .collect();
    let mut bests_by_name: HashMap<String, YearBests> = athlete_names
        .iter()
        .map(|name| {
            (
                name.clone(),
                YearBests {
                    best_snatch: 0.0,
                    best_cj: 0.0,
                    best_total: 0.0,
                },
            )
        })
        .collect();

    for row in rows {
        let bests = bests_by_name
            .entry(row.name.clone())
            .or_insert_with(|| YearBests {
                best_snatch: 0.0,
                best_cj: 0.0,
                best_total: 0.0,
            });

        bests.best_snatch = bests.best_snatch.max(max_positive([
            row.snatch_best,
            row.snatch1,
            row.snatch2,
            row.snatch3,
        ]));
        bests.best_cj = bests
            .best_cj
            .max(max_positive([row.cj_best, row.cj1, row.cj2, row.cj3]));
        bests.best_total = bests.best_total.max(row.total.max(0.0));

        recent_results_by_name
            .entry(row.name.clone())
            .or_default()
            .push(row);
    }

    let year_bests_by_name = bests_by_name.into_iter().collect();

    (recent_results_by_name, year_bests_by_name)
}

fn max_positive(values: [f64; 4]) -> f64 {
    values
        .into_iter()
        .filter(|value| *value > 0.0)
        .fold(0.0, f64::max)
}

impl From<AthletePackageRow> for PackageAthlete {
    fn from(row: AthletePackageRow) -> Self {
        let session = match (row.session_number, row.session_platform) {
            (Some(number), Some(platform)) => Some(PackageAthleteSession {
                session_number: number,
                session_platform: platform,
                date: row.date.clone(),
                start_time: row.start_time.clone(),
                weigh_in_time: row.weigh_in_time.clone(),
            }),
            _ => None,
        };

        Self {
            member_id: row.member_id,
            name: row.name,
            age: row.age,
            club: row.club,
            wso: row.wso,
            gender: row.gender,
            weight_class: row.weight_class,
            entry_total: row.entry_total,
            adaptive: row.adaptive,
            session,
        }
    }
}
