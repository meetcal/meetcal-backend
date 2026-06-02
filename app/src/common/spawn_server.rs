use tokio::net::TcpListener;

#[derive(Debug)]
pub struct TestApp {
    pub address: String,
}

pub async fn spawn_app() -> TestApp {
    crate::load_env();

    let address = "127.0.0.1:0".to_string();
    let listener = TcpListener::bind(&address).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let address = format!("http://127.0.0.1:{port}");
    let server = crate::run(listener);

    tokio::spawn(server);

    TestApp { address }
}
