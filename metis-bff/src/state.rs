use std::sync::Arc;

use metis::client::MetisClient;
use tokio::task::JoinHandle;
use tracing::warn;

use crate::cache::{self, EntityCache};
use crate::config::BffConfig;
use crate::upstream::Upstream;

/// Shared state for the BFF layer, generic over the upstream implementation.
pub struct BffState<U: Upstream> {
    pub upstream: Arc<U>,
    pub config: Arc<BffConfig>,
    pub cache: Option<Arc<EntityCache>>,
    cache_task: Option<Arc<JoinHandle<()>>>,
}

impl<U: Upstream> BffState<U> {
    pub fn new(upstream: U, config: BffConfig) -> Self {
        let upstream = Arc::new(upstream);
        let (cache, cache_task) = if config.cache_enabled {
            match Self::start_cache(&config) {
                Some((c, t)) => (Some(c), Some(Arc::new(t))),
                None => (None, None),
            }
        } else {
            (None, None)
        };

        Self {
            upstream,
            config: Arc::new(config),
            cache,
            cache_task,
        }
    }

    fn start_cache(config: &BffConfig) -> Option<(Arc<EntityCache>, JoinHandle<()>)> {
        let upstream_url = config.upstream_url.as_deref()?;
        let auth_token = config.upstream_auth_token.as_deref().unwrap_or("");

        let client = match MetisClient::new(upstream_url, auth_token) {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "failed to create MetisClient for cache, disabling cache");
                return None;
            }
        };

        let entity_cache = Arc::new(EntityCache::new());
        let handle = cache::spawn_cache_population_task(Arc::clone(&entity_cache), client);
        Some((entity_cache, handle))
    }
}

// Manual Clone implementation since Arc<U> is Clone regardless of U: Clone.
impl<U: Upstream> Clone for BffState<U> {
    fn clone(&self) -> Self {
        Self {
            upstream: Arc::clone(&self.upstream),
            config: Arc::clone(&self.config),
            cache: self.cache.clone(),
            cache_task: self.cache_task.clone(),
        }
    }
}
