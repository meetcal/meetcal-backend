use app::routes::clubs::get_athletes_by_club::ClubsAthletes;
use serde_json::Value;

mod support;

#[tokio::test]
async fn success_get_all_clubs() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/clubs", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<String> = response.json().await.unwrap();

    assert!(!body.is_empty());
}

#[tokio::test]
async fn fail_get_all_clubs() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/club", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_athletes_by_clubs() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/clubs/athletes?meet=2026%20Ohio%20WSO%20Championships&club=Columbus%20Weightlifting",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<ClubsAthletes> = response.json().await.unwrap();

    assert!(!body.is_empty());
    assert!(body.iter().all(|row| row.club == "Columbus Weightlifting"));
}

#[tokio::test]
async fn club_athletes_include_registrations_for_non_completed_meets() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/clubs/athletes?club=Vardanian%20Weightlifting",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Vec<ClubsAthletes> = response.json().await.unwrap();

    assert_eq!(body.len(), 1);
    assert_eq!(body[0].name, "Kyle Schulman");
    assert_eq!(
        body[0].meet,
        "2026 USA Weightlifting National Championships, Powered by Rogue Fitness"
    );
}

#[tokio::test]
async fn fail_get_athletes_by_clubs() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/clubs/athletes", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_club_meet_stats() {
    let app = support::spawn_test_app().await;
    let url = format!(
        "{}/clubs/meet-stats?meet=2026%20Ohio%20WSO%20Championships&club=Columbus%20Weightlifting",
        app.address
    );
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Value = response.json().await.unwrap();

    assert_eq!(body["total_athletes"], 1);
    assert_eq!(body["gold_medals"], 0);
    assert_eq!(body["silver_medals"], 0);
    assert_eq!(body["bronze_medals"], 0);
    assert_eq!(body["total_prs"], 0);
    assert_eq!(body["perfect_6_for_6"], 0);
    assert_eq!(body["snatch_make_rate"], 0);
    assert_eq!(body["cj_make_rate"], 0);
    assert_eq!(body["combined_make_rate"], 0);
    assert!(body["athlete_results"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn fail_get_club_meet_stats() {
    let app = support::spawn_test_app().await;
    let url = format!("{}/clubs/meet-stats", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn empty_club_query_is_rejected_for_strict_clients() {
    let app = support::spawn_test_app().await;
    let strict = reqwest::Client::new()
        .get(format!("{}/clubs/athletes?club=%20", app.address))
        .header("X-MeetCal-App", "6.2.0")
        .send()
        .await
        .unwrap();
    assert_eq!(strict.status(), 400);

    let legacy = reqwest::get(format!("{}/clubs/athletes?club=%20", app.address))
        .await
        .unwrap();
    assert_eq!(legacy.status(), 200);
    assert!(
        legacy
            .json::<Vec<ClubsAthletes>>()
            .await
            .unwrap()
            .is_empty()
    );
}

/// Medals are ranked within gender + weight class + division label, not the
/// bare weight class: a 71kg woman and a 71kg man from one club each win
/// their own division, so the club counts two golds per lift, not one gold
/// and one silver.
#[tokio::test]
async fn meet_stats_rank_medals_within_gender_and_division() {
    let app = support::spawn_test_app().await;
    let db = support::db_pool().await;
    let meet = "Medal Partition Test Meet";
    let club = "Medal Test Club";
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
    };
    cleanup(db.clone()).await;

    sqlx::query(
        r#"
        INSERT INTO athletes (convex_id, member_id, name, age, club, gender, weight_class,
                              entry_total, meet)
        VALUES
            ('test-medal-woman', '1', 'Medal Test Woman', 25, $1, 'Women', '71kg', 150, $2),
            ('test-medal-man', '2', 'Medal Test Man', 25, $1, 'Men', '71kg', 250, $2),
            ('test-medal-rival', '3', 'Medal Test Rival', 25, 'Other Club', 'Men', '71kg', 240, $2)
        "#,
    )
    .bind(club)
    .bind(meet)
    .execute(&db)
    .await
    .unwrap();
    sqlx::query(
        r#"
        INSERT INTO lifting_results (convex_id, event_id, meet, date, name, age, body_weight,
                                     snatch1, snatch2, snatch3, snatch_best, cj1, cj2, cj3, cj_best,
                                     total, adaptive, federation)
        VALUES
            ('test-medal-result-woman', 'm1', $1, '2026-09-01', 'Medal Test Woman',
             'Open Women''s 71kg', 70, 65, 68, 70, 70, 80, 83, 85, 85, 155, false, 'USAW'),
            ('test-medal-result-man', 'm2', $1, '2026-09-01', 'Medal Test Man',
             'Open Men''s 71kg', 70, 110, 115, 118, 118, 135, 140, 142, 142, 260, false, 'USAW'),
            ('test-medal-result-rival', 'm3', $1, '2026-09-01', 'Medal Test Rival',
             'Open Men''s 71kg', 70, 105, 110, 112, 112, 130, 135, 137, 137, 249, false, 'USAW')
        "#,
    )
    .bind(meet)
    .execute(&db)
    .await
    .unwrap();

    let response = reqwest::get(format!(
        "{}/clubs/meet-stats?meet=Medal%20Partition%20Test%20Meet&club=Medal%20Test%20Club",
        app.address
    ))
    .await
    .unwrap();
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    cleanup(db).await;

    assert_eq!(status, 200);
    assert_eq!(body["total_athletes"], 2);
    assert_eq!(body["gold_medals"], 6, "{body}");
    assert_eq!(body["silver_medals"], 0, "{body}");
    let results = body["athlete_results"].as_array().unwrap();
    assert!(
        results.iter().all(|athlete| athlete["medal"] == "gold"
            && athlete["snatch_medal"] == "gold"
            && athlete["cj_medal"] == "gold"),
        "{results:?}"
    );
}
