//! Filesystem mounts for the worker run lifecycle.
//!
//! Each [`Mount`] captures one "set up filesystem state before the agent
//! runs, then optionally persist it after" flow (repo checkout, build
//! cache, documents sync, ...). The trait has two phases — `setup` runs
//! before the agent and `save` runs after — and each [`Mount`] implementation
//! materializes a single bundle kind into the worker FS. The
//! [`orchestrator::run_phase`] helper drives a single phase end-to-end,
//! applying a timeout, routing errors as fatal or non-fatal, and emitting
//! phase-bracketed status logs, so the per-kind impls only have to express
//! what to do, not how to survive failure.

use std::time::Duration;

pub mod build_cache;
pub mod bundle;
pub mod documents;
pub mod orchestrator;
pub mod spec;

#[cfg(test)]
mod orchestration_tests;

pub use build_cache::BuildCacheMount;
pub use bundle::{bundle_mount, BundleMount};
pub use documents::DocumentsMount;

/// An error returned from a [`Mount::setup`] or [`Mount::save`] call.
///
/// The `fatal` flag tells the orchestrator how to route the failure:
///
/// - `fatal: false` → push onto the worker's `errors` vec; the session
///   ends in the `Failed` state but the worker keeps running other phases.
/// - `fatal: true` → abort the worker run immediately; no further mounts,
///   no agent.
///
/// "Best-effort" behavior (log a warning and keep going) is just
/// "return `Ok(())`" from the mount — it never returns `Err` for
/// best-effort failures.
#[derive(Debug)]
pub struct MountError {
    pub source: anyhow::Error,
    pub fatal: bool,
}

impl MountError {
    /// Push the error onto the worker's `errors` vec without aborting.
    pub fn tracked(err: impl Into<anyhow::Error>) -> Self {
        Self {
            source: err.into(),
            fatal: false,
        }
    }

    /// Abort the worker run immediately.
    pub fn fatal(err: impl Into<anyhow::Error>) -> Self {
        Self {
            source: err.into(),
            fatal: true,
        }
    }
}

pub type MountResult = std::result::Result<(), MountError>;

/// Per-phase metadata: a static label that appears in log lines, plus an
/// optional timeout. When `timeout` is `None` the orchestrator does not
/// wrap the call in `tokio::time::timeout`.
pub struct Phase {
    pub label: &'static str,
    pub timeout: Option<Duration>,
}

/// A filesystem mount with a setup phase (before the agent runs) and an
/// optional save phase (after the agent runs).
///
/// A `Mount` is **only constructed when it can actually be applied** —
/// `setup`/`save` should not contain runtime "should I skip?" checks. A
/// mount owns its target directory and is responsible for creating it
/// inside `setup` (the orchestrator does not pre-create per-mount
/// directories).
#[async_trait::async_trait]
pub trait Mount: Send {
    /// Setup-phase metadata.
    fn setup_phase(&self) -> Phase;

    /// Save-phase metadata. `None` means this mount has no post-agent
    /// phase.
    fn save_phase(&self) -> Option<Phase>;

    /// Prepare filesystem state before the agent runs.
    async fn setup(&mut self) -> MountResult;

    /// Persist filesystem state after the agent runs. Default = noop.
    async fn save(&mut self) -> MountResult {
        Ok(())
    }
}
