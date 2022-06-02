use portguard::client::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info")
    }
    env_logger::init();
    let port = std::env::args()
        .find_map(|s| s.parse::<u16>().ok()) // first valid argument
        .unwrap_or(8022); // default
    Client::run_client(port, None).await.map_err(|e| {
        log::error!("Error occured: {}", e);
        e
    })
}
