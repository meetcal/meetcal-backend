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
    pub attempt_estimates: Vec<PackageAttemptEstimateSession>,
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

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
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
struct YearBestsByNameRow {
    name: String,
    best_snatch: f64,
    best_cj: f64,
    best_total: f64,
}

#[derive(Debug, Serialize)]
pub struct PackageAttemptEstimateSession {
    pub session_number: f64,
    pub platform: String,
    pub date: Option<String>,
    pub start_time: Option<String>,
    pub weigh_in_time: Option<String>,
    pub estimates: Vec<PackageAttemptEstimate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageAttemptEstimate {
    pub athlete_id: String,
    pub athlete_name: String,
    pub weight_class: String,
    pub entry_total: f64,
    pub history_result_count: usize,
    pub snatch: PackageLiftAttemptEstimate,
    pub clean_and_jerk: PackageLiftAttemptEstimate,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageLiftAttemptEstimate {
    pub attempts: Vec<f64>,
    pub attempts_out: usize,
    pub average_increase: AttemptIncrease,
    pub make_rate: f64,
    pub historical_best: Option<f64>,
    pub source: AttemptEstimateSource,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct AttemptIncrease {
    pub first_to_second: f64,
    pub second_to_third: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptEstimateSource {
    History,
    EntryTotal,
    Unavailable,
}

#[derive(Debug, Clone, Copy)]
enum LiftType {
    Snatch,
    CleanAndJerk,
}

#[derive(Debug)]
struct AttemptData {
    athlete_id: String,
    weight: f64,
    attempt_number: usize,
}

#[derive(Debug)]
struct TempAttemptEstimate {
    athlete: PackageAthlete,
    history: Vec<PackageLiftingResult>,
    best_snatch: Option<f64>,
    best_cj: Option<f64>,
    avg_snatch_increase: AttemptIncrease,
    avg_cj_increase: AttemptIncrease,
    snatch_make_rate: f64,
    cj_make_rate: f64,
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
///   "attempt_estimates": [],
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

    let (recent_results_by_name, history_rows) = if let Some(cutoff_date) =
        params.history_cutoff_date.as_ref()
    {
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

        let (recent_results_by_name, _) = build_history_maps(&athlete_names, history_rows.clone());
        (recent_results_by_name, history_rows)
    } else {
        (BTreeMap::new(), Vec::new())
    };
    let year_bests_by_name = if params.history_cutoff_date.is_some() && !athlete_names.is_empty() {
        fetch_year_bests_by_name(&state, &athlete_names).await?
    } else {
        BTreeMap::new()
    };

    let package_athletes: Vec<PackageAthlete> =
        athletes.into_iter().map(PackageAthlete::from).collect();
    let attempt_estimates = build_attempt_estimates(&package_athletes, &history_rows);

    Ok(Json(MeetPackage {
        meet,
        schedule: build_schedule(schedule_rows),
        athletes: package_athletes,
        meet_results,
        attempt_estimates,
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

fn build_attempt_estimates(
    athletes: &[PackageAthlete],
    history_rows: &[PackageLiftingResult],
) -> Vec<PackageAttemptEstimateSession> {
    let mut sessions: Vec<PackageAttemptEstimateSession> = Vec::new();

    for athlete in athletes {
        let Some(session) = athlete.session.as_ref() else {
            continue;
        };

        let session_exists = sessions.iter().any(|estimate_session| {
            estimate_session.session_number == session.session_number
                && estimate_session.platform == session.session_platform
        });

        if !session_exists {
            sessions.push(PackageAttemptEstimateSession {
                session_number: session.session_number,
                platform: session.session_platform.clone(),
                date: session.date.clone(),
                start_time: session.start_time.clone(),
                weigh_in_time: session.weigh_in_time.clone(),
                estimates: Vec::new(),
            });
        }
    }

    for session in &mut sessions {
        let session_athletes: Vec<PackageAthlete> = athletes
            .iter()
            .filter(|athlete| {
                athlete.session.as_ref().is_some_and(|athlete_session| {
                    athlete_session.session_number == session.session_number
                        && athlete_session.session_platform == session.platform
                })
            })
            .cloned()
            .collect();
        session.estimates = build_session_attempt_estimates(&session_athletes, history_rows);
    }

    sessions.sort_by(|a, b| {
        a.session_number
            .total_cmp(&b.session_number)
            .then_with(|| a.platform.cmp(&b.platform))
    });
    sessions
}

fn build_session_attempt_estimates(
    athletes: &[PackageAthlete],
    history_rows: &[PackageLiftingResult],
) -> Vec<PackageAttemptEstimate> {
    let mut temp_estimates = Vec::new();

    for athlete in athletes {
        let normalized_name = normalize_athlete_name(&athlete.name);
        let history: Vec<PackageLiftingResult> = history_rows
            .iter()
            .filter(|row| normalize_athlete_name(&row.name) == normalized_name)
            .cloned()
            .collect();

        let best_snatch = history.iter().filter_map(snatch_best).reduce(f64::max);
        let best_cj = history.iter().filter_map(cj_best).reduce(f64::max);
        let avg_snatch_increase = calculate_average_increase(&history, LiftType::Snatch);
        let avg_cj_increase = calculate_average_increase(&history, LiftType::CleanAndJerk);
        let (snatch_make_rate, cj_make_rate) = calculate_make_rates(&history);

        temp_estimates.push(TempAttemptEstimate {
            athlete: athlete.clone(),
            history,
            best_snatch,
            best_cj,
            avg_snatch_increase,
            avg_cj_increase,
            snatch_make_rate,
            cj_make_rate,
        });
    }

    let session_avg_snatch = session_average_increase(
        temp_estimates
            .iter()
            .filter(|estimate| estimate.best_snatch.is_some())
            .map(|estimate| estimate.avg_snatch_increase),
        3.0,
    );
    let session_avg_cj = session_average_increase(
        temp_estimates
            .iter()
            .filter(|estimate| estimate.best_cj.is_some())
            .map(|estimate| estimate.avg_cj_increase),
        4.0,
    );

    let mut estimates: Vec<PackageAttemptEstimate> = temp_estimates
        .into_iter()
        .map(|estimate| {
            let snatch = estimate_lift(
                estimate.best_snatch,
                estimate.avg_snatch_increase,
                session_avg_snatch,
                estimate.athlete.entry_total,
                0.43,
                3.0,
                estimate.snatch_make_rate,
            );
            let clean_and_jerk = estimate_lift(
                estimate.best_cj,
                estimate.avg_cj_increase,
                session_avg_cj,
                estimate.athlete.entry_total,
                0.57,
                4.0,
                estimate.cj_make_rate,
            );

            PackageAttemptEstimate {
                athlete_id: estimate.athlete.member_id.clone(),
                athlete_name: estimate.athlete.name,
                weight_class: estimate.athlete.weight_class,
                entry_total: estimate.athlete.entry_total,
                history_result_count: estimate.history.len(),
                snatch,
                clean_and_jerk,
            }
        })
        .collect();

    calculate_attempts_out(&mut estimates);
    estimates.sort_by(|a, b| {
        a.snatch
            .attempts_out
            .cmp(&b.snatch.attempts_out)
            .then_with(|| {
                a.clean_and_jerk
                    .attempts_out
                    .cmp(&b.clean_and_jerk.attempts_out)
            })
            .then_with(|| a.athlete_name.cmp(&b.athlete_name))
    });
    estimates
}

fn estimate_lift(
    historical_best: Option<f64>,
    athlete_average_increase: AttemptIncrease,
    session_average_increase: AttemptIncrease,
    entry_total: f64,
    total_ratio: f64,
    default_jump: f64,
    make_rate: f64,
) -> PackageLiftAttemptEstimate {
    if let Some(best) = historical_best {
        let first_attempt = (best * 0.93).round();
        let second_attempt = first_attempt + athlete_average_increase.first_to_second;
        let third_attempt = second_attempt + athlete_average_increase.second_to_third;
        return PackageLiftAttemptEstimate {
            attempts: vec![first_attempt, second_attempt, third_attempt],
            attempts_out: 0,
            average_increase: athlete_average_increase,
            make_rate,
            historical_best: Some(best),
            source: AttemptEstimateSource::History,
        };
    }

    if entry_total > 0.0 {
        let estimated_total = (entry_total * 0.93).round();
        let first_attempt = (estimated_total * total_ratio).round();
        let second_attempt = first_attempt + session_average_increase.first_to_second;
        let third_attempt = second_attempt + session_average_increase.second_to_third;
        return PackageLiftAttemptEstimate {
            attempts: vec![first_attempt, second_attempt, third_attempt],
            attempts_out: 0,
            average_increase: session_average_increase,
            make_rate,
            historical_best: None,
            source: AttemptEstimateSource::EntryTotal,
        };
    }

    unavailable_lift_estimate(default_jump)
}

fn unavailable_lift_estimate(default_jump: f64) -> PackageLiftAttemptEstimate {
    PackageLiftAttemptEstimate {
        attempts: Vec::new(),
        attempts_out: 0,
        average_increase: AttemptIncrease {
            first_to_second: default_jump,
            second_to_third: default_jump,
        },
        make_rate: 0.0,
        historical_best: None,
        source: AttemptEstimateSource::Unavailable,
    }
}

fn calculate_attempts_out(estimates: &mut [PackageAttemptEstimate]) {
    let snatch_attempts = collect_attempts(estimates, LiftType::Snatch);
    let cj_attempts = collect_attempts(estimates, LiftType::CleanAndJerk);

    for estimate in estimates {
        estimate.snatch.attempts_out =
            attempts_out_before_first_attempt(&snatch_attempts, &estimate.athlete_id);
        estimate.clean_and_jerk.attempts_out =
            attempts_out_before_first_attempt(&cj_attempts, &estimate.athlete_id);
    }
}

fn collect_attempts(estimates: &[PackageAttemptEstimate], lift_type: LiftType) -> Vec<AttemptData> {
    let mut attempts = Vec::new();

    for estimate in estimates {
        let lift_attempts = match lift_type {
            LiftType::Snatch => &estimate.snatch.attempts,
            LiftType::CleanAndJerk => &estimate.clean_and_jerk.attempts,
        };

        for (index, weight) in lift_attempts.iter().enumerate() {
            if *weight > 0.0 {
                attempts.push(AttemptData {
                    athlete_id: estimate.athlete_id.clone(),
                    weight: *weight,
                    attempt_number: index + 1,
                });
            }
        }
    }

    attempts.sort_by(|a, b| a.weight.total_cmp(&b.weight));
    attempts
}

fn attempts_out_before_first_attempt(attempts: &[AttemptData], athlete_id: &str) -> usize {
    let Some(first_attempt_index) = attempts
        .iter()
        .position(|attempt| attempt.athlete_id == athlete_id && attempt.attempt_number == 1)
    else {
        return 0;
    };

    let mut attempts_out = 0;
    for index in 0..first_attempt_index {
        attempts_out += 1;
        if index + 1 < first_attempt_index
            && attempts[index].athlete_id == attempts[index + 1].athlete_id
        {
            attempts_out += 1;
        }
    }

    attempts_out
}

fn calculate_average_increase(
    results: &[PackageLiftingResult],
    lift_type: LiftType,
) -> AttemptIncrease {
    let mut first_to_second = Vec::new();
    let mut second_to_third = Vec::new();

    for result in results {
        let attempts = match lift_type {
            LiftType::Snatch => [result.snatch1, result.snatch2, result.snatch3],
            LiftType::CleanAndJerk => [result.cj1, result.cj2, result.cj3],
        };

        if attempts[0] > 0.0 && attempts[1] > 0.0 {
            first_to_second.push((attempts[1] - attempts[0]).abs());
        }
        if attempts[1] > 0.0 && attempts[2] > 0.0 {
            second_to_third.push((attempts[2] - attempts[1]).abs());
        }
    }

    let default_jump = match lift_type {
        LiftType::Snatch => 3.0,
        LiftType::CleanAndJerk => 4.0,
    };

    AttemptIncrease {
        first_to_second: rounded_average(&first_to_second, default_jump),
        second_to_third: rounded_average(&second_to_third, default_jump),
    }
}

fn session_average_increase(
    increases: impl Iterator<Item = AttemptIncrease>,
    default_jump: f64,
) -> AttemptIncrease {
    let increases: Vec<AttemptIncrease> = increases.collect();
    if increases.is_empty() {
        return AttemptIncrease {
            first_to_second: default_jump,
            second_to_third: default_jump,
        };
    }

    let first_to_second: Vec<f64> = increases
        .iter()
        .map(|increase| increase.first_to_second)
        .collect();
    let second_to_third: Vec<f64> = increases
        .iter()
        .map(|increase| increase.second_to_third)
        .collect();

    AttemptIncrease {
        first_to_second: rounded_average(&first_to_second, default_jump),
        second_to_third: rounded_average(&second_to_third, default_jump),
    }
}

fn rounded_average(values: &[f64], default_value: f64) -> f64 {
    if values.is_empty() {
        return default_value;
    }

    (values.iter().sum::<f64>() / values.len() as f64).round()
}

fn calculate_make_rates(results: &[PackageLiftingResult]) -> (f64, f64) {
    (
        calculate_lift_make_rate(results, LiftType::Snatch),
        calculate_lift_make_rate(results, LiftType::CleanAndJerk),
    )
}

fn calculate_lift_make_rate(results: &[PackageLiftingResult], lift_type: LiftType) -> f64 {
    let mut openers_declared = 0;
    let mut openers_made = 0;

    for result in results {
        let opener = match lift_type {
            LiftType::Snatch => result.snatch1,
            LiftType::CleanAndJerk => result.cj1,
        };

        if opener != 0.0 {
            openers_declared += 1;
            if opener > 0.0 {
                openers_made += 1;
            }
        }
    }

    if openers_declared == 0 {
        0.0
    } else {
        openers_made as f64 / openers_declared as f64
    }
}

fn snatch_best(result: &PackageLiftingResult) -> Option<f64> {
    max_successful([
        result.snatch_best,
        result.snatch1,
        result.snatch2,
        result.snatch3,
    ])
}

fn cj_best(result: &PackageLiftingResult) -> Option<f64> {
    max_successful([result.cj_best, result.cj1, result.cj2, result.cj3])
}

fn max_successful(values: [f64; 4]) -> Option<f64> {
    let best = max_positive(values);
    if best > 0.0 { Some(best) } else { None }
}

fn normalize_athlete_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
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

async fn fetch_year_bests_by_name(
    state: &AppState,
    athlete_names: &[String],
) -> Result<BTreeMap<String, YearBests>, AppError> {
    let mut bests_by_name: BTreeMap<String, YearBests> = athlete_names
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

    let rows = sqlx::query_as::<_, YearBestsByNameRow>(
        r#"
        SELECT
            name,
            COALESCE(MAX(GREATEST(
                COALESCE(snatch_best, 0),
                COALESCE(snatch1, 0),
                COALESCE(snatch2, 0),
                COALESCE(snatch3, 0)
            )), 0) AS best_snatch,
            COALESCE(MAX(GREATEST(
                COALESCE(cj_best, 0),
                COALESCE(cj1, 0),
                COALESCE(cj2, 0),
                COALESCE(cj3, 0)
            )), 0) AS best_cj,
            COALESCE(MAX(COALESCE(total, 0)), 0) AS best_total
        FROM lifting_results
        WHERE name = ANY($1::text[])
            AND date >= (CURRENT_DATE - INTERVAL '1 year')::date::text
        GROUP BY name
        "#,
    )
    .bind(athlete_names)
    .fetch_all(&state.db)
    .await?;

    for row in rows {
        bests_by_name.insert(
            row.name,
            YearBests {
                best_snatch: row.best_snatch,
                best_cj: row.best_cj,
                best_total: row.best_total,
            },
        );
    }

    Ok(bests_by_name)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempt_estimates_use_entry_total_when_history_is_missing() {
        let athletes = vec![PackageAthlete {
            member_id: "12345".to_string(),
            name: "Kyle Schulman".to_string(),
            age: 27.0,
            club: "Vardanian Weightlifting".to_string(),
            wso: None,
            gender: "Male".to_string(),
            weight_class: "+110".to_string(),
            entry_total: 365.0,
            adaptive: false,
            session: Some(PackageAthleteSession {
                session_number: 45.0,
                session_platform: "Red".to_string(),
                date: Some("2026-06-20".to_string()),
                start_time: Some("08:00:00".to_string()),
                weigh_in_time: Some("06:00:00".to_string()),
            }),
        }];

        let sessions = build_attempt_estimates(&athletes, &[]);

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_number, 45.0);
        assert_eq!(sessions[0].platform, "Red");
        assert_eq!(sessions[0].estimates.len(), 1);

        let estimate = &sessions[0].estimates[0];
        assert_eq!(estimate.athlete_name, "Kyle Schulman");
        assert_eq!(estimate.history_result_count, 0);
        assert_eq!(estimate.snatch.attempts, vec![146.0, 149.0, 152.0]);
        assert_eq!(estimate.clean_and_jerk.attempts, vec![193.0, 197.0, 201.0]);
        assert!(matches!(
            estimate.snatch.source,
            AttemptEstimateSource::EntryTotal
        ));
        assert!(matches!(
            estimate.clean_and_jerk.source,
            AttemptEstimateSource::EntryTotal
        ));
    }
}
