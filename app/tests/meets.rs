use app::routes::meets::{
    get_sessions_for_athletes::SessionsAthletes,
    types::{Athlete, MeetSchedule, Meets},
};
use serde_json::Value;

mod support;

#[tokio::test]
async fn success_get_all_meets() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/meets", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<Meets> = response.json().await.unwrap();

    assert!(!body.is_empty());
}

#[tokio::test]
async fn fail_get_all_meets() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/meet", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_meet_details() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/meets/details?meet=2026%20Ohio%20WSO%20Championships",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Meets = response.json().await.unwrap();

    assert_eq!(body.name, "2026 Ohio WSO Championships");
}

#[tokio::test]
async fn fail_get_meet_details() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/meets/", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_athletes_by_meet() {
    let app = support::spawn_test_app().await;
    let meet = "2026 USA Weightlifting National Championships, Powered by Rogue Fitness";
    let url = format!(
        "{}/meets/athletes?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<Athlete> = response.json().await.unwrap();

    assert!(!body.is_empty());
    assert!(body.iter().all(|row| row.meet == meet));
}

#[tokio::test]
async fn fail_get_athletes_by_meet() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/meets/athletes", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_meet_schedule() {
    let app = support::spawn_test_app().await;
    let meet = "2026 USA Weightlifting National Championships, Powered by Rogue Fitness";
    let url = format!(
        "{}/meets/schedule?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<MeetSchedule> = response.json().await.unwrap();

    assert!(!body.is_empty());
    assert!(body.iter().all(|row| row.meet == meet));
}

#[tokio::test]
async fn fail_get_meet_schedule() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/meets/schedule", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_sessions_for_athletes() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/meets/athletes-sessions?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<SessionsAthletes> = response.json().await.unwrap();

    assert!(!body.is_empty());
    // `success_get_sessions_for_athletes_without_schedule` runs in parallel
    // and briefly adds a session-less athlete to this meet, so only rows
    // with a session are held to the seeded session number.
    assert!(
        body.iter()
            .filter(|row| row.session_number.is_some())
            .all(|row| row.session_number == Some(45.0)),
        "{body:?}"
    );
    assert!(
        body.iter().any(|row| row.name == "Kyle Schulman"),
        "{body:?}"
    );
}

#[tokio::test]
async fn success_get_sessions_for_athletes_filtered() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/meets/athletes-sessions?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness&session_number=45&platform=Red",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<SessionsAthletes> = response.json().await.unwrap();

    assert!(!body.is_empty());
    assert!(
        body.iter().all(|row| row.session_number == Some(45.0)
            && row.session_platform.as_deref() == Some("Red"))
    );
}

#[tokio::test]
async fn success_get_sessions_for_athletes_without_schedule() {
    let app = support::spawn_test_app().await;

    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
    let db = sqlx::PgPool::connect(&database_url).await.unwrap();

    // An athlete registered before sessions are assigned must still appear
    // in the start list (LEFT JOIN), with null session fields.
    sqlx::query(
        r#"
        INSERT INTO athletes (convex_id, member_id, name, age, club, gender, weight_class, entry_total, meet)
        VALUES ('test-unassigned-athlete', '0', 'Test Unassigned', 25, 'Test Club', 'Male', '89', 200,
                '2026 USA Weightlifting National Championships, Powered by Rogue Fitness')
        ON CONFLICT (convex_id) DO NOTHING
        "#,
    )
    .execute(&db)
    .await
    .unwrap();

    let url = format!(
        "{}/meets/athletes-sessions?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();
    assert_eq!(response.status(), 200);
    let body: Vec<SessionsAthletes> = response.json().await.unwrap();

    sqlx::query("DELETE FROM athletes WHERE convex_id = 'test-unassigned-athlete'")
        .execute(&db)
        .await
        .unwrap();

    let unassigned = body
        .iter()
        .find(|row| row.name == "Test Unassigned")
        .expect("athlete without a session must be returned");
    assert_eq!(unassigned.session_number, None);
    assert_eq!(unassigned.date, None);
}

/// A meet of its own, so the rows never collide with the seeded nationals
/// assertions that run in parallel.
const LEGACY_PLATFORM_MEET: &str = "Legacy Platform Casing Test Meet";

#[tokio::test]
async fn platform_filter_matches_legacy_casing_and_whitespace() {
    let app = support::spawn_test_app().await;
    let db = support::db_pool().await;

    // Rows written before ingest canonicalised platforms: "gold " on both the
    // roster and the schedule (they were written by one run, so they agree
    // with each other but not with the app's "Gold").
    sqlx::query(
        r#"
        INSERT INTO session_schedule (convex_id, date, session_id, start_time, weigh_in_time, platform, weight_class, meet)
        VALUES ('legacy-platform-schedule', '2026-07-04', 3, '10:00 AM', '8:00 AM', 'gold ', '81', $1)
        ON CONFLICT (convex_id) DO NOTHING
        "#,
    )
    .bind(LEGACY_PLATFORM_MEET)
    .execute(&db)
    .await
    .unwrap();
    sqlx::query(
        r#"
        INSERT INTO athletes (convex_id, member_id, name, age, club, gender, weight_class, entry_total, session_number, session_platform, meet)
        VALUES ('legacy-platform-athlete', '9', 'Legacy Platform Lifter', 30, 'Test Club', 'Male', '81', 250, 3, 'gold ', $1)
        ON CONFLICT (convex_id) DO NOTHING
        "#,
    )
    .bind(LEGACY_PLATFORM_MEET)
    .execute(&db)
    .await
    .unwrap();

    let meet = LEGACY_PLATFORM_MEET.replace(' ', "%20");
    let mut bodies = Vec::new();
    for query in [
        "&platform=Gold",
        "&platform=GOLD",
        "&platform=%20gold%20%20",
        "&session_number=3&platform=Gold",
        "&session_number=3&platform=gold",
    ] {
        let url = format!("{}/meets/athletes-sessions?meet={meet}{query}", app.address);
        let response = reqwest::get(&url).await.unwrap();
        assert_eq!(response.status(), 200, "{query}");
        let body: Vec<SessionsAthletes> = response.json().await.unwrap();
        bodies.push((query, body));
    }
    let other_url = format!(
        "{}/meets/athletes-sessions?meet={meet}&platform=Red",
        app.address
    );
    let other: Vec<SessionsAthletes> = reqwest::get(&other_url)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    sqlx::query("DELETE FROM athletes WHERE meet = $1")
        .bind(LEGACY_PLATFORM_MEET)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("DELETE FROM session_schedule WHERE meet = $1")
        .bind(LEGACY_PLATFORM_MEET)
        .execute(&db)
        .await
        .unwrap();

    for (query, body) in bodies {
        assert_eq!(body.len(), 1, "{query}: {body:?}");
        assert_eq!(body[0].name, "Legacy Platform Lifter", "{query}");
        // The stored spelling is returned; the app canonicalises it.
        assert_eq!(
            body[0].session_platform.as_deref(),
            Some("gold "),
            "{query}"
        );
        assert_eq!(body[0].start_time.as_deref(), Some("10:00 AM"), "{query}");
    }
    assert!(
        other.is_empty(),
        "a different platform must not match: {other:?}"
    );
}

#[tokio::test]
async fn fail_get_sessions_for_athletes() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/meets/athletes-sessions", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_meet_package() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/meets/package?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness&history_cutoff_date=2024-01-01",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Value = response.json().await.unwrap();

    assert_eq!(
        body["meet"]["name"],
        "2026 USA Weightlifting National Championships, Powered by Rogue Fitness"
    );
    assert!(!body["schedule"].as_array().unwrap().is_empty());
    assert!(!body["athletes"].as_array().unwrap().is_empty());
    assert!(body["meet_results"].as_array().unwrap().is_empty());
    assert!(!body["attempt_estimates"].as_array().unwrap().is_empty());
    assert_eq!(body["attempt_estimates"][0]["session_number"], 45.0);
    assert_eq!(body["attempt_estimates"][0]["platform"], "Red");
    assert_eq!(
        body["attempt_estimates"][0]["estimates"][0]["athlete_name"],
        "Kyle Schulman"
    );
    assert_eq!(
        body["attempt_estimates"][0]["estimates"][0]["snatch"]["attempts"],
        serde_json::json!([146.0, 149.0, 152.0])
    );
    assert_eq!(
        body["attempt_estimates"][0]["estimates"][0]["clean_and_jerk"]["attempts"],
        serde_json::json!([193.0, 197.0, 201.0])
    );
    assert_eq!(
        body["attempt_estimates"][0]["estimates"][0]["snatch"]["source"],
        "entry_total"
    );
    assert!(body["year_bests_by_name"].is_object());
    assert!(body["recent_results_by_name"].is_object());
}

#[tokio::test]
async fn fail_get_meet_package() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/meets/package", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn unknown_meet_details_are_not_found() {
    let app = support::spawn_test_app().await;
    let response = reqwest::get(format!(
        "{}/meets/details?meet=Definitely%20Not%20A%20Meet",
        app.address
    ))
    .await
    .unwrap();
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn empty_meet_package_query_is_rejected_for_strict_clients() {
    let app = support::spawn_test_app().await;
    let strict = reqwest::Client::new()
        .get(format!("{}/meets/package?meet=%20", app.address))
        .header("X-MeetCal-App", STRICT_CLIENT)
        .send()
        .await
        .unwrap();
    assert_eq!(strict.status(), 400);
    // A legacy client gets what it always got: no such meet.
    let legacy = reqwest::get(format!("{}/meets/package?meet=%20", app.address))
        .await
        .unwrap();
    assert_eq!(legacy.status(), 404);
}

// `/meets/package` conditional requests: the body carries a strong ETag, and a
// matching `If-None-Match` answers `304` with no body.

const PACKAGE_URL: &str = "/meets/package?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness&history_cutoff_date=2024-01-01";

#[tokio::test]
async fn package_revalidates_with_etag() {
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();

    let first = client
        .get(format!("{}{PACKAGE_URL}", app.address))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 200);
    let etag = first
        .headers()
        .get(reqwest::header::ETAG)
        .expect("package carries an ETag")
        .to_str()
        .unwrap()
        .to_string();
    assert!(etag.starts_with('"') && etag.ends_with('"'), "{etag}");
    let body = first.bytes().await.unwrap();
    assert!(!body.is_empty());

    let revalidate = client
        .get(format!("{}{PACKAGE_URL}", app.address))
        .header(reqwest::header::IF_NONE_MATCH, &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(revalidate.status(), 304);
    assert_eq!(
        revalidate
            .headers()
            .get(reqwest::header::ETAG)
            .unwrap()
            .to_str()
            .unwrap(),
        etag
    );
    assert!(revalidate.bytes().await.unwrap().is_empty());

    let stale = client
        .get(format!("{}{PACKAGE_URL}", app.address))
        .header(reqwest::header::IF_NONE_MATCH, "\"not-the-etag\"")
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), 200);
    assert_eq!(
        stale
            .headers()
            .get(reqwest::header::ETAG)
            .unwrap()
            .to_str()
            .unwrap(),
        etag,
        "same body, same validator"
    );
}

// ---------------------------------------------------------------------------
// Client version gate. Blank required params were flipped to a global 400 on
// 2026-09-08; shipped app builds were built against the pre-flip answers, so
// the 400 is now gated on `X-MeetCal-App` >= 6.2.0 like every other strict
// check. Each row is (path, what a legacy client gets). Strict clients get 400
// on every gated row.
// ---------------------------------------------------------------------------

const STRICT_CLIENT: &str = "6.2.0";
const LEGACY_CLIENT: &str = "6.1.0";

#[tokio::test]
async fn blank_params_are_gated_per_endpoint() {
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let gated: [(&str, u16); 14] = [
        ("/meets/details?meet=%20", 404),
        ("/meets/schedule?meet=%20", 200),
        ("/meets/athletes?meet=%20", 200),
        ("/meets/athletes-sessions?meet=%20", 200),
        ("/meets/package?meet=%20", 404),
        ("/clubs/athletes?club=%20", 200),
        ("/clubs/meet-stats?club=%20&meet=%20", 200),
        ("/wsos/athletes?wso=%20", 200),
        ("/lifting-results?meet=%20", 200),
        ("/lifting-results/year?name=%20", 200),
        ("/search?query=%20", 200),
        ("/data/wso/records?wso=", 200),
        ("/data/wso/age-groups?wso=%20", 200),
        (
            "/data/nat-rankings-year?age_category=Open%20Men%27s%2060kg&federation=USAW&year=abc",
            200,
        ),
    ];
    for (path, legacy_status) in gated {
        for version in [None, Some(LEGACY_CLIENT)] {
            let mut request = client.get(format!("{}{path}", app.address));
            if let Some(version) = version {
                request = request.header("X-MeetCal-App", version);
            }
            let response = request.send().await.unwrap();
            assert_eq!(
                response.status(),
                legacy_status,
                "{path} legacy version={version:?}"
            );
        }
        let strict = client
            .get(format!("{}{path}", app.address))
            .header("X-MeetCal-App", STRICT_CLIENT)
            .send()
            .await
            .unwrap();
        assert_eq!(strict.status(), 400, "{path} strict");
        let body: Value = strict.json().await.unwrap();
        assert!(
            body["error"].as_str().is_some_and(|e| !e.is_empty()),
            "{path} strict body"
        );
    }

    // Always strict, for every client: these were never 200 for a shipped
    // build (name-list bounds per AGENTS.md) or are parameters no shipped
    // build sends.
    let always_400 = [
        "/lifting-results/by-names?names=",
        "/lifting-results/by-names?names=Ada&limit_per_name=0",
        "/lifting-results/by-names?names=Ada&limit_per_name=201",
        "/meets/package?meet=2026%20Ohio%20WSO%20Championships&include=schedule",
        "/meets/package?meet=2026%20Ohio%20WSO%20Championships&history_cutoff_date=nope",
        "/data/adaptive?exclude_federation=BWL&gender=Men&season=abcd",
    ];
    for path in always_400 {
        for version in [None, Some(LEGACY_CLIENT), Some(STRICT_CLIENT)] {
            let mut request = client.get(format!("{}{path}", app.address));
            if let Some(version) = version {
                request = request.header("X-MeetCal-App", version);
            }
            let response = request.send().await.unwrap();
            assert_eq!(response.status(), 400, "{path} version={version:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// Meet list: `id`, zone-aware "today", cache headers.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn meets_and_details_carry_the_convex_id() {
    let app = support::spawn_test_app().await;
    let details: Value = reqwest::get(format!(
        "{}/meets/details?meet=2026%20Ohio%20WSO%20Championships",
        app.address
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(details["id"], "meet_ohio_2026");

    let list: Vec<Value> = reqwest::get(format!("{}/meets", app.address))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!list.is_empty());
    assert!(
        list.iter()
            .all(|meet| meet["id"].as_str().is_some_and(|id| !id.is_empty())),
        "{list:?}"
    );
}

#[tokio::test]
async fn upcoming_meets_use_the_meets_own_local_date() {
    let app = support::spawn_test_app().await;
    let db = support::db_pool().await;

    // The SQL helper is the meet's local date and never errors on a zone
    // Postgres does not know.
    let (pacific, helper_pacific, bogus, utc): (String, String, String, String) = sqlx::query_as(
        "SELECT (NOW() AT TIME ZONE 'America/Los_Angeles')::date::text,
                meet_local_date('America/Los_Angeles')::text,
                meet_local_date('Not/AZone')::text,
                CURRENT_DATE::text",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(helper_pacific, pacific);
    assert_eq!(bogus, utc);

    // A Pacific meet on the last day of the three-month window in its own
    // zone, and a meet with an unknown zone: both are listed, neither 500s.
    sqlx::query(
        r#"
        INSERT INTO meets (convex_id, name, federation, start_date, end_date, status, time_zone,
                           updated_at, venue_name, venue_street, venue_city, venue_state, venue_zip)
        VALUES
            ('test-meet-pacific-edge', 'Pacific Edge Test Meet', 'USAW',
             ((NOW() AT TIME ZONE 'America/Los_Angeles')::date + INTERVAL '3 months')::date,
             ((NOW() AT TIME ZONE 'America/Los_Angeles')::date + INTERVAL '3 months')::date,
             'upcoming', 'America/Los_Angeles', 1, 'v', 's', 'c', 'CA', '90001'),
            ('test-meet-bogus-zone', 'Bogus Zone Test Meet', 'USAW',
             CURRENT_DATE, CURRENT_DATE, 'upcoming', 'Not/AZone', 1, 'v', 's', 'c', 'OH', '43215')
        ON CONFLICT (convex_id) DO NOTHING
        "#,
    )
    .execute(&db)
    .await
    .unwrap();

    let response = reqwest::get(format!("{}/meets", app.address))
        .await
        .unwrap();
    let status = response.status();
    let list: Vec<Value> = response.json().await.unwrap();

    sqlx::query(
        "DELETE FROM meets WHERE convex_id IN ('test-meet-pacific-edge', 'test-meet-bogus-zone')",
    )
    .execute(&db)
    .await
    .unwrap();

    assert_eq!(status, 200);
    let names: Vec<&str> = list.iter().filter_map(|m| m["name"].as_str()).collect();
    assert!(names.contains(&"Pacific Edge Test Meet"), "{names:?}");
    assert!(names.contains(&"Bogus Zone Test Meet"), "{names:?}");
}

#[tokio::test]
async fn schedule_carries_cache_headers_and_revalidates() {
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let url = format!(
        "{}/meets/schedule?meet=2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness",
        app.address
    );

    let first = client.get(&url).send().await.unwrap();
    assert_eq!(first.status(), 200);
    assert_eq!(
        first.headers().get(reqwest::header::CACHE_CONTROL).unwrap(),
        "no-cache"
    );
    let etag = first
        .headers()
        .get(reqwest::header::ETAG)
        .expect("schedule carries an ETag")
        .to_str()
        .unwrap()
        .to_string();
    assert!(etag.starts_with('"') && etag.ends_with('"'), "{etag}");
    let rows: Vec<MeetSchedule> = first.json().await.unwrap();
    assert!(!rows.is_empty());

    let revalidate = client
        .get(&url)
        .header(reqwest::header::IF_NONE_MATCH, &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(revalidate.status(), 304);
    assert_eq!(
        revalidate.headers().get(reqwest::header::ETAG).unwrap(),
        &etag
    );
    assert_eq!(
        revalidate
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .unwrap(),
        "no-cache"
    );
    assert!(revalidate.bytes().await.unwrap().is_empty());

    for path in [
        "/meets",
        "/meets/details?meet=2026%20Ohio%20WSO%20Championships",
    ] {
        let response = client
            .get(format!("{}{path}", app.address))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{path}");
        assert_eq!(
            response
                .headers()
                .get(reqwest::header::CACHE_CONTROL)
                .unwrap(),
            "no-cache",
            "{path}"
        );
        assert!(
            response.headers().get(reqwest::header::ETAG).is_some(),
            "{path}"
        );
    }
}

#[tokio::test]
async fn health_reports_pool_gauges() {
    let app = support::spawn_test_app().await;
    let body: Value = reqwest::get(format!("{}/health", app.address))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["status"], "ok");
    assert!(body["db"]["size"].is_u64());
    assert!(body["db"]["idle"].is_u64());
}

// ---------------------------------------------------------------------------
// Package: include sets, single-flight, ingest invalidation, bests window.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn package_include_year_bests_omits_the_other_sections_and_revalidates() {
    let app = support::spawn_test_app().await;
    let client = reqwest::Client::new();
    let url = format!("{}{PACKAGE_URL}&include=year_bests", app.address);

    let full = client
        .get(format!("{}{PACKAGE_URL}", app.address))
        .send()
        .await
        .unwrap();
    let full_etag = full.headers().get(reqwest::header::ETAG).unwrap().clone();

    let partial = client.get(&url).send().await.unwrap();
    assert_eq!(partial.status(), 200);
    let etag = partial
        .headers()
        .get(reqwest::header::ETAG)
        .unwrap()
        .clone();
    assert_ne!(etag, full_etag, "the include set is part of the validator");
    let body: Value = partial.json().await.unwrap();
    assert!(body["year_bests_by_name"].is_object());
    assert!(body["schedule"].is_array());
    assert!(body["athletes"].is_array());
    assert!(body["meet_results"].is_array());
    assert!(body.get("recent_results_by_name").is_none(), "{body}");
    assert!(body.get("attempt_estimates").is_none(), "{body}");

    let revalidate = client
        .get(&url)
        .header(reqwest::header::IF_NONE_MATCH, &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(revalidate.status(), 304);
}

#[tokio::test]
async fn concurrent_misses_build_the_package_once() {
    let app = support::spawn_test_app().await;
    let meet = "2026 Ohio WSO Championships";
    // A cutoff no other test uses, so this key starts cold.
    let url = format!(
        "{}/meets/package?meet=2026%20Ohio%20WSO%20Championships&history_cutoff_date=2023-07-07",
        app.address
    );
    let builds_before = app::routes::meets::get_meet_package::package_builds(meet);

    let client = reqwest::Client::new();
    let requests = (0..8).map(|_| {
        let client = client.clone();
        let url = url.clone();
        tokio::spawn(async move {
            let response = client.get(&url).send().await.unwrap();
            let status = response.status();
            let etag = response
                .headers()
                .get(reqwest::header::ETAG)
                .unwrap()
                .clone();
            let body = response.bytes().await.unwrap();
            (status, etag, body)
        })
    });
    let mut etags = std::collections::HashSet::new();
    for request in requests {
        let (status, etag, body) = request.await.unwrap();
        assert_eq!(status, 200);
        assert!(!body.is_empty());
        etags.insert(etag);
    }
    assert_eq!(etags.len(), 1, "every waiter got the one built body");
    assert_eq!(
        app::routes::meets::get_meet_package::package_builds(meet),
        builds_before + 1,
        "single-flight: one build for eight concurrent misses"
    );
}

#[tokio::test]
async fn package_is_rebuilt_as_soon_as_an_ingest_changes_the_meet() {
    let app = support::spawn_test_app().await;
    let db = support::db_pool().await;
    let meet = "Package Freshness Test Meet";
    let url = format!(
        "{}/meets/package?meet=Package%20Freshness%20Test%20Meet&history_cutoff_date=2024-01-01",
        app.address
    );
    let cleanup = |db: sqlx::PgPool| async move {
        sqlx::query("DELETE FROM lifting_results WHERE meet = $1")
            .bind(meet)
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("DELETE FROM athletes WHERE meet = $1")
            .bind(meet)
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("DELETE FROM meets WHERE name = $1")
            .bind(meet)
            .execute(&db)
            .await
            .unwrap();
    };
    cleanup(db.clone()).await;

    sqlx::query(
        r#"
        INSERT INTO meets (convex_id, name, federation, start_date, end_date, status, time_zone,
                           updated_at, venue_name, venue_street, venue_city, venue_state, venue_zip)
        VALUES ('test-meet-freshness', $1, 'USAW', '2026-10-01', '2026-10-02', 'upcoming',
                'America/New_York', 1, 'v', 's', 'c', 'OH', '43215')
        "#,
    )
    .bind(meet)
    .execute(&db)
    .await
    .unwrap();
    sqlx::query(
        r#"
        INSERT INTO athletes (convex_id, member_id, name, age, club, gender, weight_class,
                              entry_total, session_number, session_platform, meet)
        VALUES ('test-athlete-freshness', '1', 'Package Test Lifter', 30, 'Test Club', 'Male',
                '89', 250, 1, 'Red', $1)
        "#,
    )
    .bind(meet)
    .execute(&db)
    .await
    .unwrap();
    // History: one row inside the bests window (cutoff + 1 year = 2025-01-01
    // onward) and a heavier one before it that must not count as a year best.
    sqlx::query(
        r#"
        INSERT INTO lifting_results (convex_id, event_id, meet, date, name, age, body_weight,
                                     snatch1, snatch2, snatch3, snatch_best, cj1, cj2, cj3, cj_best,
                                     total, adaptive, federation)
        VALUES
            ('test-result-freshness-2025', 'e1', 'Old Test Meet A', '2025-03-01', 'Package Test Lifter',
             'Open Men''s 89kg', 88, 90, 95, 0, 95, 110, 0, 0, 110, 205, false, 'USAW'),
            ('test-result-freshness-2024', 'e2', 'Old Test Meet B', '2024-06-01', 'Package Test Lifter',
             'Open Men''s 89kg', 88, 100, 105, 110, 110, 130, 135, 140, 140, 250, false, 'USAW')
        ON CONFLICT (convex_id) DO NOTHING
        "#,
    )
    .execute(&db)
    .await
    .unwrap();

    let client = reqwest::Client::new();
    let first = client.get(&url).send().await.unwrap();
    assert_eq!(first.status(), 200);
    let etag1 = first.headers().get(reqwest::header::ETAG).unwrap().clone();
    let body1: Value = first.json().await.unwrap();
    assert!(body1["meet_results"].as_array().unwrap().is_empty());
    assert_eq!(
        body1["year_bests_by_name"]["Package Test Lifter"]["best_total"], 205.0,
        "bests window starts one year after the caller's cutoff: {body1}"
    );
    assert_eq!(
        body1["recent_results_by_name"]["Package Test Lifter"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(body1["recent_results_by_name"]["Package Test Lifter"][0]["id"].is_i64());
    assert!(body1["recent_results_by_name"]["Package Test Lifter"][0]["event_id"].is_string());

    // Cached: same validator.
    let again = client.get(&url).send().await.unwrap();
    assert_eq!(again.headers().get(reqwest::header::ETAG).unwrap(), &etag1);

    // An ingest writes this meet's own results. No TTL wait: the next request
    // sees them and the validator changes.
    sqlx::query(
        r#"
        INSERT INTO lifting_results (convex_id, event_id, meet, date, name, age, body_weight,
                                     snatch1, snatch2, snatch3, snatch_best, cj1, cj2, cj3, cj_best,
                                     total, adaptive, federation)
        VALUES ('test-result-freshness-meet', 'e3', $1, '2026-10-01', 'Package Test Lifter',
                'Open Men''s 89kg', 88, 95, 100, 0, 100, 120, 0, 0, 120, 220, false, 'USAW')
        "#,
    )
    .bind(meet)
    .execute(&db)
    .await
    .unwrap();

    let after = client
        .get(&url)
        .header(reqwest::header::IF_NONE_MATCH, &etag1)
        .send()
        .await
        .unwrap();
    let status = after.status();
    let etag2 = after.headers().get(reqwest::header::ETAG).unwrap().clone();
    let body2: Value = after.json().await.unwrap();
    cleanup(db).await;

    assert_eq!(status, 200, "stale validator must not 304 after an ingest");
    assert_ne!(etag2, etag1);
    assert_eq!(body2["meet_results"].as_array().unwrap().len(), 1);
    assert_eq!(
        app::routes::meets::get_meet_package::package_builds(meet),
        2,
        "one build before the ingest, one after"
    );
}

#[tokio::test]
async fn test_database_has_every_migration_this_build_embeds() {
    // The same check `main` runs before serving: a build must refuse a
    // database that is behind it rather than 500 on the routes that need
    // the missing schema.
    let db = support::db_pool().await;
    app::common::schema::ensure_migrations_applied(&db)
        .await
        .expect("seeded test database is migrated");
}
