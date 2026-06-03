use crate::configuration::get_configuration;
use sqlx::PgPool;
use tokio::net::TcpListener;

#[derive(Debug)]
pub struct TestApp {
    pub address: String,
}

pub async fn spawn_app() -> TestApp {
    crate::load_env();

    let config = get_configuration().expect("Failed to read config");
    let db = PgPool::connect(
        &config
            .database
            .connection_string()
            .expect("Failed to build database connection string"),
    )
    .await
    .expect("Failed to connect to postgres");

    let address = "127.0.0.1:0".to_string();
    let listener = TcpListener::bind(&address).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let address = format!("http://127.0.0.1:{port}");
    let server = crate::run(listener, db);

    tokio::spawn(server);

    TestApp { address }
}
