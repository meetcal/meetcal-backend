use app::configuration::get_configuration;
use app::{load_env, run};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use tokio::net::TcpListener;

/// Postgres pool sizing. The ceiling is the number of concurrent statements
/// this process can have in flight; the floor keeps warm connections so a
/// meet-weekend burst does not pay TCP + TLS setup on every request.
const MAX_DB_CONNECTIONS: u32 = 20;
const MIN_DB_CONNECTIONS: u32 = 2;
/// How long a request waits for a free pool connection before failing. Shorter
/// than the 15s request timeout so the pool, not the client, reports saturation.
const DB_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() {
    load_env();

    let config = get_configuration().expect("Failed to read config");
    let connection = PgPoolOptions::new()
        .max_connections(MAX_DB_CONNECTIONS)
        .min_connections(MIN_DB_CONNECTIONS)
        .acquire_timeout(DB_ACQUIRE_TIMEOUT)
        .connect(
            &config
                .database
                .connection_string()
                .expect("Failed to build database connection string"),
        )
        .await
        .expect("Failed to connect to postgres");

    let address = format!("{}:{}", config.application_host, config.application_port);
    let listener = TcpListener::bind(address).await.unwrap();
    run(listener, connection).await;
}
