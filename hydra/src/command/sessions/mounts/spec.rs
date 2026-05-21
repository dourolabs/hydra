//! `MountSpec` → `Vec<Box<dyn Mount>>` adapter.
//!
//! The wire-level [`MountSpec`] / [`MountItem`] types describe **what** to
//! mount; this module decides **how** by mapping each [`MountItem`] to the
//! corresponding mount constructor in this crate. See
//! `/designs/worker-context-mount-spec.md` for the full design.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use hydra_common::sessions::{MountItem, MountSpec, RelativePath};

use crate::client::HydraClientInterface;

use super::build_cache::BuildCacheMount;
use super::bundle::bundle_mount;
use super::documents::DocumentsMount;
use super::Mount;

/// Runtime inputs `instantiate` needs that cannot come from the spec itself.
///
/// `session_id` and `issue_branch_id` are deliberately **not** in this struct
/// — they ride on the corresponding [`MountItem`] variants so the server can
/// stamp them at spec construction time.
pub struct InstantiateInputs<'a> {
    pub github_token: Option<String>,
    pub worker_home_dir: Option<PathBuf>,
    pub dest: &'a Path,
    pub client: Arc<dyn HydraClientInterface>,
}

/// Result of [`instantiate`]: the agent's CWD plus the ordered list of mounts
/// the worker should drive through `setup` and `save`.
pub struct InstantiatedMounts {
    pub working_dir: PathBuf,
    pub mounts: Vec<Box<dyn Mount>>,
}

/// Failure modes for [`instantiate`].
#[derive(Debug)]
pub enum MountSpecError {
    /// A `RelativePath` somehow holds an absolute path or `..` component
    /// despite the constructor / deserializer check. Defensive only.
    InvalidPath,
    /// The spec contains a [`MountItem::Unknown`] — the server is asking for
    /// a mount kind this client does not understand. Fatal.
    UnsupportedItem,
    /// Semantic validation failure (e.g. an unsupported bundle variant).
    Validation(String),
}

impl std::fmt::Display for MountSpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPath => f.write_str(
                "MountSpec contained a path that is absolute or escapes `dest`",
            ),
            Self::UnsupportedItem => f.write_str(
                "MountSpec contained an unsupported MountItem variant (client too old for this spec)",
            ),
            Self::Validation(reason) => write!(f, "MountSpec validation failed: {reason}"),
        }
    }
}

impl std::error::Error for MountSpecError {}

/// Build the agent CWD and the ordered mount list from a [`MountSpec`].
///
/// The returned `mounts` is in the same order as `spec.mounts` — `setup` runs
/// front-to-back, `save` runs in the same order on the way out. A
/// [`MountItem::Unknown`] is fatal (no fallback to the legacy path); the
/// caller's `mount_spec = None` branch is the only legacy-path entry point.
pub fn instantiate(
    spec: &MountSpec,
    inputs: InstantiateInputs<'_>,
) -> Result<InstantiatedMounts, MountSpecError> {
    let working_dir = resolve_under_dest(inputs.dest, &spec.working_dir)?;

    let mut mounts: Vec<Box<dyn Mount>> = Vec::with_capacity(spec.mounts.len());
    for item in &spec.mounts {
        match item {
            MountItem::Bundle {
                target,
                bundle,
                session_id,
                issue_branch_id,
            } => {
                let repo_path = resolve_under_dest(inputs.dest, target)?;
                let mount = bundle_mount(
                    bundle,
                    repo_path,
                    inputs.github_token.clone(),
                    session_id.clone(),
                    issue_branch_id.clone(),
                )
                .map_err(|err| MountSpecError::Validation(err.to_string()))?;
                mounts.push(Box::new(mount));
            }
            MountItem::BuildCache {
                repo_target,
                service_repo_name,
                context,
                session_id: _,
            } => {
                let repo_path = resolve_under_dest(inputs.dest, repo_target)?;
                mounts.push(Box::new(BuildCacheMount::new(
                    repo_path,
                    inputs.worker_home_dir.clone(),
                    context.clone(),
                    service_repo_name.clone(),
                )));
            }
            MountItem::Documents { target } => {
                let documents_path = resolve_under_dest(inputs.dest, target)?;
                mounts.push(Box::new(DocumentsMount::new(
                    documents_path,
                    Arc::clone(&inputs.client),
                )));
            }
            // `MountItem` is `#[non_exhaustive]`; `Unknown` plus any future
            // variant the server might emit is treated as fatal until the
            // client learns about it.
            MountItem::Unknown => return Err(MountSpecError::UnsupportedItem),
            _ => return Err(MountSpecError::UnsupportedItem),
        }
    }

    Ok(InstantiatedMounts {
        working_dir,
        mounts,
    })
}

/// First `MountItem::Documents` target in the spec, or `None`.
///
/// Used by `worker_run` to pin `HYDRA_DOCUMENTS_DIR` before any mount setup
/// runs, mirroring the legacy pre-mount env var injection.
pub fn find_documents_dir(spec: &MountSpec) -> Option<&RelativePath> {
    spec.mounts.iter().find_map(|item| match item {
        MountItem::Documents { target } => Some(target),
        _ => None,
    })
}

/// Defensive re-check on top of `RelativePath`'s own validation: refuse to
/// resolve any path with absolute / `..` / root components, so a malformed
/// path that somehow slipped past deserialization cannot escape `dest`.
fn resolve_under_dest(dest: &Path, rel: &RelativePath) -> Result<PathBuf, MountSpecError> {
    for component in rel.as_path().components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return Err(MountSpecError::InvalidPath);
            }
        }
    }
    Ok(dest.join(rel.as_path()))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use hydra_common::sessions::{Bundle, MountItem, MountSpec, RelativePath};
    use hydra_common::{BuildCacheContext, BuildCacheSettings, BuildCacheStorageConfig, RepoName};
    use reqwest::Client as HttpClient;

    use crate::client::HydraClient;
    use crate::command::sessions::mounts::{self, Mount};
    use crate::test_utils::ids::task_id;

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

    fn dummy_bundle() -> Bundle {
        Bundle::GitRepository {
            url: "https://example.com/acme/widgets".to_string(),
            rev: "main".to_string(),
        }
    }

    fn rel(s: &str) -> RelativePath {
        RelativePath::new(s).expect("relative path")
    }

    /// Per-mount "fingerprint" used to assert that two mount lists are
    /// equivalent without exposing each mount's internals.
    ///
    /// The setup-phase label is set per concrete `Mount` impl and is the
    /// quickest way to distinguish `BundleMount` ("repo checkout") from
    /// `BuildCacheMount` ("cache apply") and `DocumentsMount` ("document
    /// sync") without a downcast.
    fn fingerprints(mounts: &[Box<dyn Mount>]) -> Vec<(&'static str, Option<&'static str>)> {
        mounts
            .iter()
            .map(|m| {
                let setup = m.setup_phase().label;
                let save = m.save_phase().map(|p| p.label);
                (setup, save)
            })
            .collect()
    }

    /// `instantiate` on the standard 3-item spec must produce the same
    /// concrete mount types in the same order as `mounts::build_mounts`,
    /// pointing at the same target paths.
    #[test]
    fn three_item_spec_matches_build_mounts() {
        let dest = PathBuf::from("/tmp/example-dest");
        let job = task_id("t-spec-3");
        let issue_branch_id = Some("i-spec-3".to_string());
        let github_token = Some("ghp_xxx".to_string());
        let worker_home_dir = Some(PathBuf::from("/tmp/worker-home"));
        let bundle = dummy_bundle();
        let cache_ctx = dummy_cache_context();
        let repo_name = dummy_repo_name();

        let spec = MountSpec::new(
            rel("repo"),
            vec![
                MountItem::Bundle {
                    target: rel("repo"),
                    bundle: bundle.clone(),
                    session_id: job.clone(),
                    issue_branch_id: issue_branch_id.clone(),
                },
                MountItem::BuildCache {
                    repo_target: rel("repo"),
                    service_repo_name: repo_name.clone(),
                    context: cache_ctx.clone(),
                    session_id: job.clone(),
                },
                MountItem::Documents {
                    target: rel("documents"),
                },
            ],
        );

        let from_spec = instantiate(
            &spec,
            InstantiateInputs {
                github_token: github_token.clone(),
                worker_home_dir: worker_home_dir.clone(),
                dest: &dest,
                client: dummy_client(),
            },
        )
        .expect("instantiate");

        assert_eq!(from_spec.working_dir, dest.join("repo"));
        assert_eq!(from_spec.mounts.len(), 3);

        let legacy = mounts::build_mounts(
            &dest.join("repo"),
            &dest.join("documents"),
            dummy_client(),
            &bundle,
            Some(&cache_ctx),
            Some(&repo_name),
            github_token,
            issue_branch_id,
            worker_home_dir,
            job,
        )
        .expect("build_mounts");

        assert_eq!(
            fingerprints(&from_spec.mounts),
            fingerprints(&legacy),
            "spec-instantiated mounts must match legacy build_mounts in order and per-mount phase labels"
        );
        assert_eq!(
            fingerprints(&from_spec.mounts),
            vec![
                ("repo checkout", Some("git finalize")),
                ("cache apply", Some("cache upload")),
                // `DocumentsMount::save_phase` is `None` until `setup` flips
                // its `synced` flag, which only happens at runtime — both
                // pre-setup snapshots agree here.
                ("document sync", None),
            ],
        );
    }

    /// A spec with no `BuildCache` item produces just `[Bundle, Documents]`
    /// — same shape as the legacy `build_mounts` output for a bundle-only run.
    #[test]
    fn two_item_spec_skips_build_cache() {
        let dest = PathBuf::from("/tmp/example-dest");
        let job = task_id("t-spec-2");
        let spec = MountSpec::new(
            rel("repo"),
            vec![
                MountItem::Bundle {
                    target: rel("repo"),
                    bundle: dummy_bundle(),
                    session_id: job,
                    issue_branch_id: None,
                },
                MountItem::Documents {
                    target: rel("documents"),
                },
            ],
        );

        let result = instantiate(
            &spec,
            InstantiateInputs {
                github_token: None,
                worker_home_dir: None,
                dest: &dest,
                client: dummy_client(),
            },
        )
        .expect("instantiate");

        assert_eq!(result.working_dir, dest.join("repo"));
        assert_eq!(result.mounts.len(), 2);
        assert_eq!(
            fingerprints(&result.mounts),
            vec![
                ("repo checkout", Some("git finalize")),
                ("document sync", None),
            ],
        );
    }

    /// `MountItem::Unknown` must be fatal — `instantiate` returns
    /// `UnsupportedItem` rather than silently skipping it.
    #[test]
    fn unknown_item_is_fatal() {
        let dest = PathBuf::from("/tmp/example-dest");
        let job = task_id("t-spec-unknown");
        let spec = MountSpec::new(
            rel("repo"),
            vec![
                MountItem::Bundle {
                    target: rel("repo"),
                    bundle: dummy_bundle(),
                    session_id: job,
                    issue_branch_id: None,
                },
                MountItem::Unknown,
                MountItem::Documents {
                    target: rel("documents"),
                },
            ],
        );

        let result = instantiate(
            &spec,
            InstantiateInputs {
                github_token: None,
                worker_home_dir: None,
                dest: &dest,
                client: dummy_client(),
            },
        );
        match result {
            Ok(_) => panic!("unknown item must abort instantiate"),
            Err(err) => assert!(
                matches!(err, MountSpecError::UnsupportedItem),
                "unexpected error variant: {err:?}"
            ),
        }
    }

    /// Even with a clean `RelativePath`, the resolved path must live under
    /// `dest`. Asserted via `starts_with` rather than canonicalization so the
    /// test stays portable (the dest tempdir need not yet exist on disk).
    #[test]
    fn resolved_paths_stay_under_dest() {
        let dest = PathBuf::from("/tmp/example-dest");

        let good = RelativePath::new("repo").expect("valid");
        let resolved = resolve_under_dest(&dest, &good).expect("ok");
        assert!(resolved.starts_with(&dest));
        assert_eq!(resolved, dest.join("repo"));

        let nested = RelativePath::new("a/b/c").expect("valid");
        let resolved_nested = resolve_under_dest(&dest, &nested).expect("ok");
        assert!(resolved_nested.starts_with(&dest));
    }

    #[test]
    fn find_documents_dir_returns_first_documents_target() {
        let job = task_id("t-find-docs");
        let spec = MountSpec::new(
            rel("repo"),
            vec![
                MountItem::Bundle {
                    target: rel("repo"),
                    bundle: dummy_bundle(),
                    session_id: job,
                    issue_branch_id: None,
                },
                MountItem::Documents {
                    target: rel("documents"),
                },
            ],
        );
        let found = find_documents_dir(&spec).expect("documents item present");
        assert_eq!(found.as_path(), Path::new("documents"));
    }

    #[test]
    fn find_documents_dir_none_when_absent() {
        let job = task_id("t-find-no-docs");
        let spec = MountSpec::new(
            rel("repo"),
            vec![MountItem::Bundle {
                target: rel("repo"),
                bundle: dummy_bundle(),
                session_id: job,
                issue_branch_id: None,
            }],
        );
        assert!(find_documents_dir(&spec).is_none());
    }

    /// `Bundle::Unknown` flows through `bundle_mount` as an error and
    /// surfaces as `MountSpecError::Validation` from `instantiate`.
    #[test]
    fn unknown_bundle_variant_is_validation_error() {
        let dest = PathBuf::from("/tmp/example-dest");
        let job = task_id("t-spec-unknown-bundle");
        let spec = MountSpec::new(
            rel("repo"),
            vec![MountItem::Bundle {
                target: rel("repo"),
                bundle: Bundle::Unknown,
                session_id: job,
                issue_branch_id: None,
            }],
        );

        let result = instantiate(
            &spec,
            InstantiateInputs {
                github_token: None,
                worker_home_dir: None,
                dest: &dest,
                client: dummy_client(),
            },
        );
        match result {
            Ok(_) => panic!("Bundle::Unknown must abort instantiate"),
            Err(err) => assert!(
                matches!(err, MountSpecError::Validation(_)),
                "unexpected error variant: {err:?}"
            ),
        }
    }
}
