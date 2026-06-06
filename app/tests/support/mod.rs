use app::common::spawn_server::{TestApp, spawn_app};

pub async fn spawn_test_app() -> TestApp {
    spawn_app().await
}
