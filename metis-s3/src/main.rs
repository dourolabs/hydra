use anyhow::Result;
use metis_s3::{build_router, config, config::AppConfig};
use tokio::net::TcpListener;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    run().await
}

async fn run() -> Result<()> {
    tracing_subscriber::fmt::init();

    let config_path = config::config_path();
    let app_config = AppConfig::load(&config_path)?;
    let bind_addr = app_config.bind_addr();
    let storage_root = app_config.storage_root();
    let request_body_limit_bytes = app_config.server.request_body_limit_bytes;

    info!(
        bind_addr = %bind_addr,
        storage_root = %storage_root.display(),
        request_body_limit_bytes = request_body_limit_bytes,
        "metis-s3 configuration loaded"
    );

    let app = build_router(storage_root.clone(), request_body_limit_bytes);
    let listener = TcpListener::bind(&bind_addr).await?;
    let addr = listener.local_addr()?;

    info!("metis-s3 listening on http://{}", addr);
    println!("metis-s3 listening on http://{addr}");

    axum::serve(listener, app).await?;

    Ok(())
}
