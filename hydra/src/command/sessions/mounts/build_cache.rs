//! `BuildCacheMount` — per-bundle [`Mount`] impl for the shared build cache.
//!
//! On `setup` the mount pulls the cache for the run's `BuildCacheContext`
//! key from the build-cache service and applies it into the repo directory
//! the bundle mount has already populated. On `save` it reads HEAD from
//! disk via [`resolve_head_oid`] and uploads a fresh cache snapshot keyed
//! to that commit, retrying up to [`BUILD_CACHE_UPLOAD_MAX_ATTEMPTS`] times
//! on transient failure.
//!
//! The mount operates on a repo directory the bundle mount owns (it does
//! not `mkdir` `repo_path` itself); re-mount of the same context onto a
//! directory that already has the cache applied is a no-op overwrite, so
//! the apply phase is idempotent on retry. Reading HEAD in `save` rather
//! than caching it from `setup` means the upload always reflects the
//! agent's final commit, not the pre-agent one.

use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Result};
use hydra_build_cache::{ApplyCacheTimings, UploadCacheTimings};
use hydra_common::{BuildCacheContext, RepoName};
use tracing::{info, warn};

use crate::build_cache::build_cache_client;
use crate::git::resolve_head_oid;

use super::{Mount, MountResult, Phase};

/// Per-attempt timeout for uploading the build cache. Mirrors the prior
/// `worker_run.rs` constant so retry semantics stay byte-identical.
pub const BUILD_CACHE_UPLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// Maximum number of upload attempts on transient failure. Mirrors the
/// `MAX_ATTEMPTS` from the inline implementation in `worker_run.rs`.
pub const BUILD_CACHE_UPLOAD_MAX_ATTEMPTS: u32 = 3;

pub struct BuildCacheMount {
    repo_path: PathBuf,
    worker_home_dir: Option<PathBuf>,
    cache_context: BuildCacheContext,
    service_repo_name: RepoName,
    downloaded_cache_sha: Option<String>,
}

impl BuildCacheMount {
    pub fn new(
        repo_path: PathBuf,
        worker_home_dir: Option<PathBuf>,
        cache_context: BuildCacheContext,
        service_repo_name: RepoName,
    ) -> Self {
        Self {
            repo_path,
            worker_home_dir,
            cache_context,
            service_repo_name,
            downloaded_cache_sha: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn downloaded_cache_sha(&self) -> Option<&str> {
        self.downloaded_cache_sha.as_deref()
    }
}

#[async_trait::async_trait]
impl Mount for BuildCacheMount {
    fn setup_phase(&self) -> Phase {
        Phase {
            label: "cache apply",
            timeout: None,
        }
    }

    fn save_phase(&self) -> Option<Phase> {
        Some(Phase {
            label: "cache upload",
            timeout: Some(BUILD_CACHE_UPLOAD_TIMEOUT),
        })
    }

    async fn setup(&mut self) -> MountResult {
        let client = match build_cache_client(&self.cache_context) {
            Ok(client) => client,
            Err(err) => {
                warn!("Build cache apply skipped: {err}");
                return Ok(());
            }
        };
        match client
            .apply_nearest_cache(
                &self.repo_path,
                self.worker_home_dir.as_deref(),
                self.service_repo_name.clone(),
            )
            .await
        {
            Ok((Some(key), timings)) => {
                info!(
                    "Build cache download/apply completed (applied entry '{}').",
                    key.object_key()
                );
                log_apply_cache_timings(&timings);
                self.downloaded_cache_sha = Some(key.git_sha.clone());
            }
            Ok((None, timings)) => {
                info!("Build cache download/apply completed (no entry found).");
                log_apply_cache_timings(&timings);
            }
            Err(err) => {
                warn!("Build cache download/apply skipped: {err}");
            }
        }
        Ok(())
    }

    async fn save(&mut self) -> MountResult {
        let head_oid = match resolve_head_oid(&self.repo_path) {
            Ok(Some(oid)) => oid,
            Ok(None) => {
                warn!("Build cache upload skipped: HEAD is unavailable.");
                return Ok(());
            }
            Err(err) => {
                warn!("Build cache upload skipped: failed to resolve HEAD: {err}");
                return Ok(());
            }
        };
        let git_sha = head_oid.to_string();
        if self.downloaded_cache_sha.as_deref() == Some(git_sha.as_str()) {
            info!("Build cache upload skipped (cache entry already up-to-date).");
            return Ok(());
        }
        let client = match build_cache_client(&self.cache_context) {
            Ok(client) => client,
            Err(err) => {
                warn!("Build cache upload skipped: {err}");
                return Ok(());
            }
        };
        let repo_path = self.repo_path.clone();
        let worker_home = self.worker_home_dir.clone();
        let service_repo_name = self.service_repo_name.clone();
        let git_sha_for_upload = git_sha.clone();
        let upload = move || {
            let client = client.clone();
            let repo_path = repo_path.clone();
            let worker_home = worker_home.clone();
            let service_repo_name = service_repo_name.clone();
            let git_sha = git_sha_for_upload.clone();
            async move {
                client
                    .build_and_upload_cache(
                        &repo_path,
                        worker_home.as_deref(),
                        service_repo_name,
                        &git_sha,
                    )
                    .await
            }
        };
        let result = upload_with_retry(
            BUILD_CACHE_UPLOAD_MAX_ATTEMPTS,
            BUILD_CACHE_UPLOAD_TIMEOUT,
            upload,
        )
        .await;
        match result {
            Ok((key, timings)) => {
                info!(
                    "Build cache create/upload completed (uploaded entry '{}').",
                    key.object_key()
                );
                log_upload_cache_timings(&timings);
            }
            Err(err) => {
                warn!(
                    "Build cache create/upload skipped after {BUILD_CACHE_UPLOAD_MAX_ATTEMPTS} attempts: {err}"
                );
            }
        }
        Ok(())
    }
}

/// Generic retry loop used by the upload phase.
///
/// Each attempt is bounded by `attempt_timeout` and the loop runs up to
/// `max_attempts` times with the same exponential backoff
/// (`2^attempt` seconds) the inline implementation used. Returning a
/// generic `Result<T>` keeps this helper independent of
/// `hydra_build_cache::BuildCacheClient` so tests can drive it with
/// any closure that satisfies the trait bounds.
async fn upload_with_retry<F, Fut, T, E>(
    max_attempts: u32,
    attempt_timeout: Duration,
    mut upload: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<T, E>>,
    E: Into<anyhow::Error>,
{
    let mut last_error: Option<anyhow::Error> = None;
    for attempt in 1..=max_attempts {
        info!("Uploading build cache (attempt {attempt}/{max_attempts})...");
        match tokio::time::timeout(attempt_timeout, upload()).await {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(err)) => {
                let err: anyhow::Error = err.into();
                warn!("Build cache upload attempt {attempt}/{max_attempts} failed: {err}");
                last_error = Some(err);
            }
            Err(_) => {
                let secs = attempt_timeout.as_secs();
                warn!(
                    "Build cache upload attempt {attempt}/{max_attempts} timed out after {secs}s"
                );
                last_error = Some(anyhow!("build cache upload timed out after {secs}s"));
            }
        }
        if attempt < max_attempts {
            let delay_secs = 2u64.pow(attempt);
            info!("Retrying build cache upload in {delay_secs}s...");
            tokio::time::sleep(Duration::from_secs(delay_secs)).await;
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("build cache upload failed with unknown error")))
}

fn log_apply_cache_timings(timings: &ApplyCacheTimings) {
    info!("  list_caches: {:.2}s", timings.list_caches.as_secs_f64());
    info!("  find_nearest: {:.2}s", timings.find_nearest.as_secs_f64());
    if let Some(dl) = &timings.download {
        info!(
            "  download: {:.2}s ({} bytes)",
            dl.elapsed.as_secs_f64(),
            dl.file_size_bytes
        );
    }
    if let Some(apply) = &timings.apply {
        info!("  apply: {:.2}s", apply.as_secs_f64());
    }
}

fn log_upload_cache_timings(timings: &UploadCacheTimings) {
    info!(
        "  build_archive: {:.2}s ({} bytes)",
        timings.build_archive.elapsed.as_secs_f64(),
        timings.build_archive.file_size_bytes
    );
    info!("  upload: {:.2}s", timings.upload.as_secs_f64());
    info!("  evict: {:.2}s", timings.evict.as_secs_f64());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{commit_changes, configure_repo, stage_all_changes};
    use anyhow::Context;
    use git2::Repository;
    use hydra_common::{BuildCacheSettings, BuildCacheStorageConfig};
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tempfile::tempdir;

    fn init_repo(path: &Path) -> Result<String> {
        Repository::init(path).context("init repo")?;
        configure_repo(path, "Test User", "test@example.com")?;
        std::fs::write(path.join("README.md"), "initial").context("write readme")?;
        stage_all_changes(path)?;
        commit_changes(path, "initial commit")?;
        Ok(resolve_head_oid(path)?
            .ok_or_else(|| anyhow!("HEAD not resolvable after initial commit"))?
            .to_string())
    }

    fn filesystem_cache_context(root: &Path) -> BuildCacheContext {
        BuildCacheContext {
            storage: BuildCacheStorageConfig::FileSystem {
                root_dir: root.to_string_lossy().into_owned(),
            },
            settings: BuildCacheSettings::default(),
        }
    }

    fn make_repo_name() -> RepoName {
        RepoName::new("acme", "widgets").expect("repo name")
    }

    #[tokio::test]
    async fn setup_records_downloaded_cache_sha_on_success() -> Result<()> {
        let repo_dir = tempdir().context("repo dir")?;
        let storage_dir = tempdir().context("storage dir")?;
        let git_sha = init_repo(repo_dir.path())?;

        // Drop an artifact into target/ and prime the cache so the
        // subsequent apply has a non-empty entry to download.
        let target_dir = repo_dir.path().join("target");
        std::fs::create_dir_all(&target_dir).context("create target dir")?;
        std::fs::write(target_dir.join("artifact.txt"), "cached").context("write artifact")?;

        let cache_context = filesystem_cache_context(storage_dir.path());
        let repo_name = make_repo_name();
        let prime_client =
            crate::build_cache::build_cache_client(&cache_context).context("client")?;
        prime_client
            .build_and_upload_cache(repo_dir.path(), None, repo_name.clone(), &git_sha)
            .await
            .context("seed cache")?;
        std::fs::remove_dir_all(&target_dir).context("clear target")?;

        let mut mount = BuildCacheMount::new(
            repo_dir.path().to_path_buf(),
            None,
            cache_context,
            repo_name,
        );
        mount.setup().await.expect("setup ok");

        assert_eq!(mount.downloaded_cache_sha(), Some(git_sha.as_str()));
        assert!(
            target_dir.join("artifact.txt").exists(),
            "cache entry should have been applied to disk"
        );
        Ok(())
    }

    #[tokio::test]
    async fn setup_returns_ok_when_cache_client_construction_fails() -> Result<()> {
        let repo_dir = tempdir().context("repo dir")?;
        init_repo(repo_dir.path())?;

        // Empty root_dir trips `FileSystemStorageConfig::validate` and
        // surfaces as a build_cache_client failure — exactly the error
        // branch the design says we swallow with a warn + Ok.
        let bad_context = BuildCacheContext {
            storage: BuildCacheStorageConfig::FileSystem {
                root_dir: String::new(),
            },
            settings: BuildCacheSettings::default(),
        };

        let mut mount = BuildCacheMount::new(
            repo_dir.path().to_path_buf(),
            None,
            bad_context,
            make_repo_name(),
        );
        let result = mount.setup().await;
        assert!(result.is_ok(), "client failure must surface as warn + Ok");
        assert!(mount.downloaded_cache_sha().is_none());
        Ok(())
    }

    #[tokio::test]
    async fn save_short_circuits_when_head_matches_downloaded_sha() -> Result<()> {
        let repo_dir = tempdir().context("repo dir")?;
        let storage_dir = tempdir().context("storage dir")?;
        let git_sha = init_repo(repo_dir.path())?;

        let cache_context = filesystem_cache_context(storage_dir.path());
        let mut mount = BuildCacheMount::new(
            repo_dir.path().to_path_buf(),
            None,
            cache_context,
            make_repo_name(),
        );
        // Pretend setup populated downloaded_cache_sha with the current
        // HEAD; save must skip the upload entirely.
        mount.downloaded_cache_sha = Some(git_sha.clone());

        let snapshot_before = list_storage_entries(storage_dir.path())?;
        mount.save().await.expect("save ok");
        let snapshot_after = list_storage_entries(storage_dir.path())?;
        assert_eq!(
            snapshot_before, snapshot_after,
            "save must not write to the cache when HEAD == downloaded_cache_sha"
        );
        Ok(())
    }

    fn list_storage_entries(root: &Path) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        if root.exists() {
            visit_files(root, &mut out)?;
        }
        out.sort();
        Ok(out)
    }

    fn visit_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(dir).context("read storage dir")? {
            let entry = entry.context("read storage entry")?;
            let file_type = entry.file_type().context("file type")?;
            let path = entry.path();
            if file_type.is_dir() {
                visit_files(&path, out)?;
            } else if file_type.is_file() {
                out.push(path);
            }
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn upload_with_retry_returns_final_error_after_persistent_failure() -> Result<()> {
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_closure = Arc::clone(&attempts);
        let result: Result<(), anyhow::Error> = upload_with_retry(
            BUILD_CACHE_UPLOAD_MAX_ATTEMPTS,
            Duration::from_secs(1),
            move || {
                let attempts = Arc::clone(&attempts_for_closure);
                async move {
                    let n = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                    Err::<(), anyhow::Error>(anyhow!("transient failure #{n}"))
                }
            },
        )
        .await;

        let err = result.expect_err("must propagate the final error");
        assert!(
            err.to_string().contains("transient failure"),
            "unexpected error: {err}"
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            BUILD_CACHE_UPLOAD_MAX_ATTEMPTS as usize,
            "must run exactly MAX_ATTEMPTS times before giving up"
        );
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn upload_with_retry_succeeds_on_third_attempt() -> Result<()> {
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_closure = Arc::clone(&attempts);
        let result = upload_with_retry(
            BUILD_CACHE_UPLOAD_MAX_ATTEMPTS,
            Duration::from_secs(1),
            move || {
                let attempts = Arc::clone(&attempts_for_closure);
                async move {
                    let n = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                    if n < BUILD_CACHE_UPLOAD_MAX_ATTEMPTS as usize {
                        Err::<&'static str, anyhow::Error>(anyhow!("flake {n}"))
                    } else {
                        Ok("done")
                    }
                }
            },
        )
        .await;

        assert_eq!(result.expect("eventually succeeds"), "done");
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            BUILD_CACHE_UPLOAD_MAX_ATTEMPTS as usize,
            "should stop retrying once the call succeeds"
        );
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn upload_with_retry_treats_timeout_as_failure() -> Result<()> {
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_closure = Arc::clone(&attempts);
        let result: Result<(), anyhow::Error> = upload_with_retry(
            BUILD_CACHE_UPLOAD_MAX_ATTEMPTS,
            Duration::from_millis(10),
            move || {
                let attempts = Arc::clone(&attempts_for_closure);
                async move {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    Ok::<(), anyhow::Error>(())
                }
            },
        )
        .await;

        let err = result.expect_err("timeouts exhaust the retry budget");
        assert!(
            err.to_string().contains("timed out"),
            "unexpected error: {err}"
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            BUILD_CACHE_UPLOAD_MAX_ATTEMPTS as usize,
            "every attempt should have started"
        );
        Ok(())
    }
}
