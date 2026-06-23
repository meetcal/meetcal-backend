use app::configuration::get_configuration;
use app::{load_env, run};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    load_env();

    let config = get_configuration().expect("Failed to read config");
    let connection = PgPoolOptions::new()
        .max_connections(20)
        .min_connections(2)
        .acquire_timeout(Duration::from_secs(5))
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
