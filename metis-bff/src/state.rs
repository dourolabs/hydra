use std::sync::Arc;

use crate::config::BffConfig;
use crate::upstream::Upstream;

/// Shared state for the BFF layer, generic over the upstream implementation.
pub struct BffState<U: Upstream> {
    pub upstream: Arc<U>,
    pub config: Arc<BffConfig>,
}

impl<U: Upstream> BffState<U> {
    pub fn new(upstream: U, config: BffConfig) -> Self {
        Self {
            upstream: Arc::new(upstream),
            config: Arc::new(config),
        }
    }
}

// Manual Clone implementation since Arc<U> is Clone regardless of U: Clone.
impl<U: Upstream> Clone for BffState<U> {
    fn clone(&self) -> Self {
        Self {
            upstream: Arc::clone(&self.upstream),
            config: Arc::clone(&self.config),
        }
    }
}
