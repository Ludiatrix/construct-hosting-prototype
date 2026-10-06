use construct_registry::{App, Config};
use std::{env, path::PathBuf};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config {
        data_dir: PathBuf::from(env::var("CONSTRUCT_DATA_DIR").unwrap_or_else(|_| "data".into())),
        python: env::var("CONSTRUCT_PYTHON").unwrap_or_else(|_| "python3".into()),
        validator: PathBuf::from(
            env::var("CONSTRUCT_VALIDATOR").unwrap_or_else(|_| "validate_usd.py".into()),
        ),
        api_key: env::var("CONSTRUCT_API_KEY")
            .map_err(|_| "set CONSTRUCT_API_KEY before starting")?,
    };
    if !config.validator.is_file() {
        return Err("USD validator script not found".into());
    }
    let available = tokio::process::Command::new(&config.python)
        .args(["-c", "from pxr import Sdf, Usd, UsdGeom, UsdUtils"])
        .status()
        .await?;
    if !available.success() {
        return Err("install requirements.txt in CONSTRUCT_PYTHON's environment".into());
    }
    let app = App::open(config)?.router();
    let bind = env::var("CONSTRUCT_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("Construct registry listening on {bind}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
