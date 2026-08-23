use crate::configuration::get_configuration;
use crate::routes::users::auth::AuthVerifier;
use sqlx::postgres::PgPoolOptions;
use std::{sync::Arc, time::Duration};
use tokio::net::TcpListener;

#[derive(Debug)]
pub struct TestApp {
    pub address: String,
}

pub async fn spawn_app() -> TestApp {
    let auth = AuthVerifier::from_env().expect("Invalid Clerk authentication configuration");
    spawn_app_with_auth(auth).await
}

pub async fn spawn_app_with_auth(auth: Option<Arc<AuthVerifier>>) -> TestApp {
    crate::load_env();

    let database_url = match std::env::var("DATABASE_URL") {
        Ok(database_url) => database_url,
        Err(_) => {
            let config = get_configuration().expect("Failed to read config");
            config
                .database
                .connection_string()
                .expect("Failed to build database connection string")
        }
    };

    let db = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&database_url)
        .await
        .expect("Failed to connect to postgres");

    let address = "127.0.0.1:0".to_string();
    let listener = TcpListener::bind(&address).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let address = format!("http://127.0.0.1:{port}");
    let server = crate::run_with_auth(listener, db, auth);

    tokio::spawn(server);

    TestApp { address }
}
