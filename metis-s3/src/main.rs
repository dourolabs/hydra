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

    info!(
        bind_addr = %bind_addr,
        storage_root = %storage_root.display(),
        "metis-s3 configuration loaded"
    );

    let app = build_router(storage_root.clone());
    let listener = TcpListener::bind(&bind_addr).await?;
    let addr = listener.local_addr()?;

    info!("metis-s3 listening on http://{}", addr);
    println!("metis-s3 listening on http://{addr}");

    axum::serve(listener, app).await?;

    Ok(())
}
