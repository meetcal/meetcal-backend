use crate::common::rate_limit::{ApiKeys, RateLimitSettings};
use crate::configuration::get_configuration;
use crate::routes::users::auth::AuthVerifier;
use sqlx::postgres::PgPoolOptions;
use std::{sync::Arc, time::Duration};
use tokio::net::TcpListener;

#[derive(Debug)]
pub struct TestApp {
    pub address: String,
}

/// The default limits: shadow mode (nothing is ever rate limited) and the
/// production in-flight cap, far above what one test sends.
pub async fn spawn_app_with_auth(auth: Option<Arc<AuthVerifier>>) -> TestApp {
    spawn_app_as(auth, false, RateLimitSettings::default(), ApiKeys::none()).await
}

/// A server with the given rate limits and API keys (`name:secret,...`, as in
/// `APP_RATE_LIMIT__KEYS`), for the rate-limit and load-shedding tests.
pub async fn spawn_app_with_limits(limits: RateLimitSettings, api_keys: &str) -> TestApp {
    let keys = ApiKeys::parse(api_keys).expect("valid test API keys");
    spawn_app_as(None, false, limits, keys).await
}

/// Like [`spawn_app_with_auth`], but every pool connection runs
/// `SET ROLE meetcal_api` first, so the routes execute with that role's
/// grants and row-level security, as production does.
pub async fn spawn_app_as_api_role(auth: Option<Arc<AuthVerifier>>) -> TestApp {
    spawn_app_as(auth, true, RateLimitSettings::default(), ApiKeys::none()).await
}

async fn spawn_app_as(
    auth: Option<Arc<AuthVerifier>>,
    as_api_role: bool,
    limits: RateLimitSettings,
    keys: ApiKeys,
) -> TestApp {
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

    let mut options = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5));
    if as_api_role {
        options = options.after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("SET ROLE meetcal_api")
                    .execute(conn)
                    .await
                    .map(|_| ())
            })
        });
    }
    let db = options
        .connect(&database_url)
        .await
        .expect("Failed to connect to postgres");

    let address = "127.0.0.1:0".to_string();
    let listener = TcpListener::bind(&address).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let address = format!("http://127.0.0.1:{port}");
    tokio::spawn(async move { crate::run_with_auth(listener, db, auth, &limits, keys).await });

    TestApp { address }
}
