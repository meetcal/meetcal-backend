use app::{
    common::spawn_server,
    routes::meets::{
        get_all_meets::MeetsList3Months,
        types::{Athlete, MeetSchedule, Meets},
    },
};

#[tokio::test]
async fn success_get_all_meets() {
    let app = spawn_server::spawn_app().await;
    let url = format!("{}/meets", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: MeetsList3Months = response.json().await.unwrap();

    assert!(!body.names.is_empty());
}

#[tokio::test]
async fn fail_get_all_meets() {
    let app = spawn_server::spawn_app().await;
    let url = format!("{}/meet", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_meet_details() {
    let app = spawn_server::spawn_app().await;
    let url = format!("{}/meets/2026%20Ohio%20WSO%20Championships", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_eq!(response.status(), 200);

    let body: Meets = response.json().await.unwrap();

    assert_eq!(body.name, "2026 Ohio WSO Championships");
}

#[tokio::test]
async fn fail_get_meet_details() {
    let app = spawn_server::spawn_app().await;
    let url = format!("{}/meets/", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_athletes_by_meet() {
    let app = spawn_server::spawn_app().await;
    let meet = "2026 USA Weightlifting National Championships, Powered by Rogue Fitness";
    let url = format!(
        "{}/meets/athletes/2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness",
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
    let app = spawn_server::spawn_app().await;
    let url = format!("{}/meets/athletes", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}

#[tokio::test]
async fn success_get_meet_schedule() {
    let app = spawn_server::spawn_app().await;
    let meet = "2026 USA Weightlifting National Championships, Powered by Rogue Fitness";
    let url = format!(
        "{}/meets/schedule/2026%20USA%20Weightlifting%20National%20Championships%2C%20Powered%20by%20Rogue%20Fitness",
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
    let app = spawn_server::spawn_app().await;
    let url = format!("{}/meets/schedule", app.address);
    let response = reqwest::get(&url).await.unwrap();

    assert_ne!(response.status(), 200);
}
