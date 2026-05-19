//! Filesystem mounts for the worker run lifecycle.
//!
//! Each [`Mount`] captures one "set up filesystem state before the agent
//! runs, then optionally persist it after" flow (repo checkout, build
//! cache, documents sync, ...). The [`orchestrator::run_phase`] helper
//! drives a single phase with timeout + error routing + phase-bracketed
//! status logging.
//!
//! See `/designs/worker-mount-trait.md` for the full design.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use hydra_common::{sessions::Bundle, BuildCacheContext, RepoName, SessionId};

use crate::client::HydraClientInterface;

pub mod build_cache;
pub mod bundle;
pub mod documents;
pub mod orchestrator;

#[cfg(test)]
mod orchestration_tests;

pub use build_cache::{build_cache_mount, BuildCacheMount};
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

/// Build the ordered list of mounts applicable to a worker run.
///
/// The order is `[BundleMount, BuildCacheMount?, DocumentsMount]`, matching
/// the setup/save sequencing described in `/designs/worker-mount-trait.md`.
/// Mounts that cannot be applied (e.g. no build cache configured) are
/// simply not constructed — there is no runtime gating inside `setup` or
/// `save`.
#[allow(clippy::too_many_arguments)]
pub fn build_mounts(
    repo_path: &Path,
    documents_path: &Path,
    client: Arc<dyn HydraClientInterface>,
    request_context: &Bundle,
    build_cache: Option<&BuildCacheContext>,
    service_repo_name: Option<&RepoName>,
    github_token: Option<String>,
    issue_branch_id: Option<String>,
    worker_home_dir: Option<PathBuf>,
    session_id: SessionId,
) -> Result<Vec<Box<dyn Mount>>> {
    let mut mounts: Vec<Box<dyn Mount>> = Vec::new();

    let bundle = bundle_mount(
        request_context,
        repo_path.to_path_buf(),
        github_token,
        session_id,
        issue_branch_id,
    )?;
    mounts.push(Box::new(bundle));

    if let Some(cache_mount) = build_cache_mount(
        request_context,
        build_cache,
        service_repo_name,
        repo_path.to_path_buf(),
        worker_home_dir,
    ) {
        mounts.push(Box::new(cache_mount));
    }

    mounts.push(Box::new(DocumentsMount::new(
        documents_path.to_path_buf(),
        client,
    )));

    Ok(mounts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::HydraClient;
    use crate::test_utils::ids::task_id;
    use hydra_common::{BuildCacheSettings, BuildCacheStorageConfig};
    use reqwest::Client as HttpClient;
    use std::path::PathBuf;

    fn dummy_client() -> Arc<dyn HydraClientInterface> {
        Arc::new(
            HydraClient::with_http_client("http://example.invalid", "tok", HttpClient::new())
                .expect("dummy client"),
        )
    }

    fn dummy_cache_context() -> BuildCacheContext {
        BuildCacheContext {
            storage: BuildCacheStorageConfig::FileSystem {
                root_dir: "/tmp/dummy-cache".into(),
            },
            settings: BuildCacheSettings::default(),
        }
    }

    fn dummy_repo_name() -> RepoName {
        RepoName::new("acme", "widgets").expect("repo name")
    }

    #[test]
    fn build_mounts_bundle_none_skips_build_cache() {
        let mounts = build_mounts(
            &PathBuf::from("/tmp/repo"),
            &PathBuf::from("/tmp/documents"),
            dummy_client(),
            &Bundle::None,
            Some(&dummy_cache_context()),
            Some(&dummy_repo_name()),
            None,
            None,
            None,
            task_id("t-bm-none"),
        )
        .expect("build_mounts");
        assert_eq!(
            mounts.len(),
            2,
            "Bundle::None must produce BundleMount + DocumentsMount (no BuildCacheMount)"
        );
    }

    #[test]
    fn build_mounts_git_repository_without_cache_skips_build_cache() {
        let bundle = Bundle::GitRepository {
            url: "https://example.com/acme/widgets".to_string(),
            rev: "main".to_string(),
        };
        let mounts = build_mounts(
            &PathBuf::from("/tmp/repo"),
            &PathBuf::from("/tmp/documents"),
            dummy_client(),
            &bundle,
            None,
            Some(&dummy_repo_name()),
            None,
            None,
            None,
            task_id("t-bm-nocache"),
        )
        .expect("build_mounts");
        assert_eq!(
            mounts.len(),
            2,
            "GitRepository without build_cache must skip BuildCacheMount"
        );
    }

    #[test]
    fn build_mounts_git_repository_with_cache_pushes_all_three() {
        let bundle = Bundle::GitRepository {
            url: "https://example.com/acme/widgets".to_string(),
            rev: "main".to_string(),
        };
        let mounts = build_mounts(
            &PathBuf::from("/tmp/repo"),
            &PathBuf::from("/tmp/documents"),
            dummy_client(),
            &bundle,
            Some(&dummy_cache_context()),
            Some(&dummy_repo_name()),
            Some("ghp_token".to_string()),
            Some("i-bm-all".to_string()),
            Some(PathBuf::from("/tmp/worker-home")),
            task_id("t-bm-all"),
        )
        .expect("build_mounts");
        assert_eq!(
            mounts.len(),
            3,
            "all three inputs present → BundleMount + BuildCacheMount + DocumentsMount"
        );
    }

    #[test]
    fn build_mounts_git_repository_without_repo_name_skips_build_cache() {
        let bundle = Bundle::GitRepository {
            url: "https://example.com/acme/widgets".to_string(),
            rev: "main".to_string(),
        };
        let mounts = build_mounts(
            &PathBuf::from("/tmp/repo"),
            &PathBuf::from("/tmp/documents"),
            dummy_client(),
            &bundle,
            Some(&dummy_cache_context()),
            None,
            None,
            None,
            None,
            task_id("t-bm-nonameid"),
        )
        .expect("build_mounts");
        assert_eq!(
            mounts.len(),
            2,
            "without service_repo_name the build cache mount is skipped"
        );
    }

    #[test]
    fn build_mounts_unknown_bundle_is_an_error() {
        let result = build_mounts(
            &PathBuf::from("/tmp/repo"),
            &PathBuf::from("/tmp/documents"),
            dummy_client(),
            &Bundle::Unknown,
            None,
            None,
            None,
            None,
            None,
            task_id("t-bm-unknown"),
        );
        assert!(result.is_err(), "Bundle::Unknown must surface as an error");
    }
}
