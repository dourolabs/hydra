use std::sync::Arc;

use metis::client::MetisClient;
use tokio::task::JoinHandle;
use tracing::warn;

use axum_extra::extract::cookie::CookieJar;

use crate::auth::COOKIE_NAME;
use crate::cache::{self, EntityCache};
use crate::config::BffConfig;
use crate::upstream::Upstream;

/// Shared state for the BFF layer, generic over the upstream implementation.
pub struct BffState<U: Upstream> {
    pub upstream: Arc<U>,
    pub config: Arc<BffConfig>,
    pub cache: Option<Arc<EntityCache>>,
    cache_task: Option<Arc<JoinHandle<()>>>,
    /// When set, the BFF injects this token as Bearer auth on all proxied
    /// requests instead of extracting from cookies (single-player mode).
    pub auto_login_token: Option<Arc<String>>,
}

impl<U: Upstream> BffState<U> {
    pub fn new(upstream: U, config: BffConfig, auto_login_token: Option<String>) -> Self {
        let upstream = Arc::new(upstream);
        let (cache, cache_task) = match &config.cache {
            Some(cache_config) => match Self::start_cache(cache_config) {
                Some((c, t)) => (Some(c), Some(Arc::new(t))),
                None => (None, None),
            },
            None => (None, None),
        };

        Self {
            upstream,
            config: Arc::new(config),
            cache,
            cache_task,
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

    fn start_cache(
        cache_config: &crate::config::CacheConfig,
    ) -> Option<(Arc<EntityCache>, JoinHandle<()>)> {
        let upstream_url = &cache_config.upstream_url;
        let auth_token = cache_config.upstream_auth_token.as_deref().unwrap_or("");

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
            auto_login_token: self.auto_login_token.clone(),
        }
    }
}
