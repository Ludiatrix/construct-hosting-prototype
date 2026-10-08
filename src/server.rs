use crate::{App, Config};

/// Initialize dependencies, bind HTTP, drain requests on shutdown, then drop resources.
pub async fn run(config: Config, bind: &str) -> Result<(), Box<dyn std::error::Error>> {
    let app = App::open(config).await?;
    app.check_dependencies().await?;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    println!("Construct registry listening on {}", listener.local_addr()?);
    axum::serve(listener, app.router())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! {
            result = tokio::signal::ctrl_c() => { result.expect("listen for Ctrl-C"); }
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await.expect("listen for Ctrl-C");
}
