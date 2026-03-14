use std::path::PathBuf;

/// How frontend assets are served by the BFF.
#[derive(Debug, Clone)]
pub enum FrontendAssets {
    /// Assets compiled into the binary via `rust-embed` (single-player).
    Embedded,
    /// Assets served from a filesystem directory (multi-player / Docker).
    Directory(PathBuf),
    /// No frontend serving (API-only mode).
    None,
}

/// Configuration for the upstream entity cache.
/// Presence of this config means the cache is enabled.
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Base URL of the upstream metis-server for the cache's SSE subscription.
    pub upstream_url: String,

    /// Auth token for the cache's SSE subscription to the upstream server.
    pub upstream_auth_token: Option<String>,
}

/// Configuration for the BFF layer.
#[derive(Debug, Clone)]
pub struct BffConfig {
    /// When set, the `/auth/login` endpoint is enabled and validates tokens
    /// against the upstream `/v1/whoami` endpoint before setting cookies.
    /// When `None`, the login endpoint returns 404.
    pub auth_login_enabled: bool,

    /// Whether to set the `Secure` flag on auth cookies.
    pub cookie_secure: bool,

    /// Frontend asset serving mode.
    pub frontend_assets: FrontendAssets,

    /// In-memory entity cache configuration.
    /// When `Some`, a background task subscribes to the upstream SSE stream
    /// and maintains an in-memory cache of entity state.
    /// When `None`, the cache is disabled.
    pub cache: Option<CacheConfig>,
}

impl Default for BffConfig {
    fn default() -> Self {
        Self {
            auth_login_enabled: true,
            cookie_secure: false,
            frontend_assets: FrontendAssets::None,
            cache: None,
        }
    }
}
