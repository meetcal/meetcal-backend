use app::configuration::get_configuration;
use app::{load_env, run};
use sqlx::PgPool;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    load_env();

    let config = get_configuration().expect("Failed to read config");
    let connection = PgPool::connect(
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
