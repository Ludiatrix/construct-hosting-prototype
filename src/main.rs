use construct_registry::{server, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    let bind = std::env::var("CONSTRUCT_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    server::run(config, &bind).await
}
