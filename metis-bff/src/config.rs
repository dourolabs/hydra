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

    /// Whether the in-memory entity cache is enabled.
    /// When enabled, a background task subscribes to the upstream SSE stream
    /// and maintains an in-memory cache of entity state.
    pub cache_enabled: bool,

    /// Auth token for the cache's SSE subscription to the upstream server.
    /// Required when cache is enabled in HTTP (multi-player) mode.
    pub upstream_auth_token: Option<String>,
}

impl Default for BffConfig {
    fn default() -> Self {
        Self {
            auth_login_enabled: true,
            cookie_secure: false,
            frontend_assets: FrontendAssets::None,
            cache_enabled: false,
            upstream_auth_token: None,
        }
    }
}
