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
    /// Whether to set the `Secure` flag on auth cookies.
    pub cookie_secure: bool,

    /// Frontend asset serving mode.
    pub frontend_assets: FrontendAssets,
}

impl Default for BffConfig {
    fn default() -> Self {
        Self {
            cookie_secure: false,
            frontend_assets: FrontendAssets::None,
        }
    }
}
