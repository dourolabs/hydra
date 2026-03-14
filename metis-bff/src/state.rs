use std::sync::Arc;

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
    pub fn new(upstream: U, config: BffConfig) -> Self {
        Self {
            upstream: Arc::new(upstream),
            config: Arc::new(config),
            auto_login_token: None,
        }
    }

    pub fn with_auto_login_token(mut self, token: String) -> Self {
        self.auto_login_token = Some(Arc::new(token));
        self
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
