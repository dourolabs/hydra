use std::net::SocketAddr;
use std::path::PathBuf;

use metis_bff::{BffConfig, BffState, FrontendAssets, HttpUpstream};
use tracing::info;

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let upstream_url =
        std::env::var("UPSTREAM_URL").expect("UPSTREAM_URL environment variable is required");

    let cookie_secure = env_or("COOKIE_SECURE", "true")
        .parse::<bool>()
        .expect("COOKIE_SECURE must be true or false");

    let frontend_assets = match std::env::var("FRONTEND_ASSETS_DIR") {
        Ok(dir) => FrontendAssets::Directory(PathBuf::from(dir)),
        Err(_) => FrontendAssets::None,
    };

    let port: u16 = env_or("PORT", "4000")
        .parse()
        .expect("PORT must be a valid port number");

    let cache_enabled = env_or("CACHE_ENABLED", "false")
        .parse::<bool>()
        .expect("CACHE_ENABLED must be true or false");

    let upstream_auth_token = std::env::var("UPSTREAM_AUTH_TOKEN").ok();

    let upstream = HttpUpstream::new(upstream_url.clone());

    let config = BffConfig {
        auth_login_enabled: true,
        cookie_secure,
        frontend_assets,
        cache_enabled,
        upstream_url: Some(upstream_url.clone()),
        upstream_auth_token,
    };

    info!(upstream_url = %upstream_url, port = port, cache_enabled = cache_enabled, "starting metis-bff-server");

    let state = BffState::new(upstream, config);
    let router = metis_bff::build_bff_router(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind listener");

    info!("listening on {addr}");
    axum::serve(listener, router).await.expect("server error");
}
