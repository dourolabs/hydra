//! Shared runtime state for the proxy subdomain router.
//!
//! Today this is just a per-target concurrent-connection cap, but the
//! type is shaped to host future additions (rate-limit state, target
//! liveness probes) without churning callers.

use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::cookie::ProxyTargetId;

/// Cap on simultaneous in-flight proxy requests per target. Reasonable
/// default for dev-server traffic; bursts over this return 503 so a
/// runaway client can't exhaust the hydra-server worker pool.
pub const DEFAULT_PER_TARGET_CONCURRENT_CAP: usize = 32;

/// Returned by [`ProxyState::try_acquire`] when the per-target cap is
/// exhausted. Carries no data — there's exactly one shape of failure here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProxyCapExhausted;

/// Shared runtime state across all proxy router invocations.
#[derive(Clone)]
pub struct ProxyState {
    inner: Arc<ProxyStateInner>,
}

struct ProxyStateInner {
    per_target_cap: usize,
    semaphores: DashMap<String, Arc<Semaphore>>,
}

impl Default for ProxyState {
    fn default() -> Self {
        Self::new(DEFAULT_PER_TARGET_CONCURRENT_CAP)
    }
}

impl ProxyState {
    pub fn new(per_target_cap: usize) -> Self {
        Self {
            inner: Arc::new(ProxyStateInner {
                per_target_cap,
                semaphores: DashMap::new(),
            }),
        }
    }

    pub fn per_target_cap(&self) -> usize {
        self.inner.per_target_cap
    }

    /// Try to acquire a permit for `target`. Returns `Ok(permit)` on
    /// success, `Err(ProxyCapExhausted)` when the cap is exhausted
    /// (caller should 503).
    pub fn try_acquire(
        &self,
        target: &ProxyTargetId,
    ) -> Result<OwnedSemaphorePermit, ProxyCapExhausted> {
        let key = target.as_label().to_string();
        let sem = self
            .inner
            .semaphores
            .entry(key)
            .or_insert_with(|| Arc::new(Semaphore::new(self.inner.per_target_cap)))
            .clone();
        sem.try_acquire_owned().map_err(|_| ProxyCapExhausted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hydra_common::ConversationId;

    #[test]
    fn try_acquire_returns_err_after_cap_exhausted() {
        let state = ProxyState::new(2);
        let target = ProxyTargetId::Conversation(ConversationId::new());
        let _p1 = state.try_acquire(&target).unwrap();
        let _p2 = state.try_acquire(&target).unwrap();
        let p3 = state.try_acquire(&target);
        assert!(p3.is_err());
    }

    #[test]
    fn dropping_permit_frees_slot() {
        let state = ProxyState::new(1);
        let target = ProxyTargetId::Conversation(ConversationId::new());
        let p1 = state.try_acquire(&target).unwrap();
        drop(p1);
        let _p2 = state
            .try_acquire(&target)
            .expect("freed slot should be reusable");
    }

    #[test]
    fn caps_are_per_target() {
        let state = ProxyState::new(1);
        let target_a = ProxyTargetId::Conversation(ConversationId::new());
        let target_b = ProxyTargetId::Conversation(ConversationId::new());
        let _pa = state.try_acquire(&target_a).unwrap();
        let _pb = state.try_acquire(&target_b).unwrap();
    }
}
