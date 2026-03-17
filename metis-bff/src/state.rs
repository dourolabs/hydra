use std::sync::Arc;

use axum_extra::extract::cookie::CookieJar;

use crate::auth::COOKIE_NAME;
use crate::config::BffConfig;
use crate::upstream::Upstream;

/// Shared state for the BFF layer, generic over the upstream implementation.
pub struct BffState<U: Upstream> {
    pub upstream: Arc<U>,
    pub config: Arc<BffConfig>,
    /// When set, the BFF injects this token as Bearer auth on all proxied
    /// requests instead of extracting from cookies (single-player mode).
    pub auto_login_token: Option<Arc<String>>,
}

impl<U: Upstream> BffState<U> {
    pub fn new(upstream: U, config: BffConfig, auto_login_token: Option<String>) -> Self {
        Self {
            upstream: Arc::new(upstream),
            config: Arc::new(config),
            auto_login_token: auto_login_token.map(Arc::new),
        }
    }

    /// Resolve the auth token: use auto_login_token if set, otherwise extract from cookie.
    pub fn resolve_token(&self, jar: &CookieJar) -> Option<String> {
        if let Some(token) = &self.auto_login_token {
            return Some(token.as_ref().clone());
        }
        jar.get(COOKIE_NAME).map(|c| c.value().to_string())
    }
}

// Manual Clone implementation since Arc<U> is Clone regardless of U: Clone.
impl<U: Upstream> Clone for BffState<U> {
    fn clone(&self) -> Self {
        Self {
            upstream: Arc::clone(&self.upstream),
            config: Arc::clone(&self.config),
            auto_login_token: self.auto_login_token.clone(),
        }
    }
}
