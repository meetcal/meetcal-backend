use app::{load_env, run};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    load_env();
    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    run(listener).await;
}
