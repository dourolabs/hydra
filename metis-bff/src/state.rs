use std::sync::Arc;

use crate::cache::{self, EntityCache};
use crate::config::BffConfig;
use crate::upstream::Upstream;

/// Shared state for the BFF layer, generic over the upstream implementation.
pub struct BffState<U: Upstream> {
    pub upstream: Arc<U>,
    pub config: Arc<BffConfig>,
    pub cache: Option<Arc<EntityCache>>,
}

impl<U: Upstream> BffState<U> {
    pub fn new(upstream: U, config: BffConfig) -> Self {
        let upstream = Arc::new(upstream);
        let cache = if config.cache_enabled {
            let entity_cache = Arc::new(EntityCache::new());
            cache::spawn_cache_population_task(
                Arc::clone(&entity_cache),
                Arc::clone(&upstream),
                config.upstream_auth_token.clone(),
            );
            Some(entity_cache)
        } else {
            None
        };

        Self {
            upstream,
            config: Arc::new(config),
            cache,
        }
    }
}

// Manual Clone implementation since Arc<U> is Clone regardless of U: Clone.
impl<U: Upstream> Clone for BffState<U> {
    fn clone(&self) -> Self {
        Self {
            upstream: Arc::clone(&self.upstream),
            config: Arc::clone(&self.config),
            cache: self.cache.clone(),
        }
    }
}
