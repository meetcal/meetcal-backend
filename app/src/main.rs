use app::common::schema::ensure_migrations_applied;
use app::configuration::get_configuration;
use app::{DB_ACQUIRE_TIMEOUT, MAX_DB_CONNECTIONS, MIN_DB_CONNECTIONS, load_env, run};
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

/// Log filter when `RUST_LOG` is unset: request spans and handler errors at
/// INFO and above, library internals quiet.
const DEFAULT_LOG_FILTER: &str = "info";

#[tokio::main]
async fn main() {
    load_env();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER)),
        )
        .init();

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

    // Exit (non-zero) rather than serve 500s from a schema this build does
    // not match; the deploy's health check then keeps the old container.
    if let Err(reason) = ensure_migrations_applied(&connection).await {
        tracing::error!("refusing to start: {reason}");
        std::process::exit(1);
    }

    let address = format!("{}:{}", config.application_host, config.application_port);
    let listener = TcpListener::bind(address).await.unwrap();
    run(listener, connection, config.rate_limit).await;
}
