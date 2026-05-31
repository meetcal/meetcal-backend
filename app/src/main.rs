use std::path::PathBuf;

use app::{
    AppState,
    routes::meets::{
        get_all_meets::list_meets_next_3months, get_athletes_by_meet::get_athletes_by_meet,
        get_meet_details::get_meet_details, get_meet_schedule::get_meet_schedule,
    },
};
use axum::{Router, routing::get};
use convex::ConvexClient;
use tokio::net::TcpListener;
use tower_http::compression::CompressionLayer;

fn load_env() {
    let env_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.env");
    dotenvy::from_path(env_path).expect("failed to load meetcal-backend/.env");
}

#[tokio::main]
async fn main() {
    load_env();

    let convex_url =
        std::env::var("CONVEX_URL").expect("CONVEX_URL must be set in meetcal-backend/.env");

    let convex = ConvexClient::new(&convex_url)
        .await
        .expect("failed to connect to Convex");

    let app = Router::new()
        .route("/meets", get(list_meets_next_3months))
        .route("/meets/{name}", get(get_meet_details))
        .route("/meets/schedule/{name}", get(get_meet_schedule))
        .route("/meets/athletes/{name}", get(get_athletes_by_meet))
        .layer(CompressionLayer::new())
        .with_state(AppState { convex });

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
