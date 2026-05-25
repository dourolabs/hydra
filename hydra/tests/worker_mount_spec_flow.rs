//! End-to-end exercise of the `MountSpec → instantiate → setup/save`
//! flow. Bypasses the harness server (which doesn't yet populate
//! `mount_spec`; see Phase 1b in `/designs/worker-context-mount-spec.md`)
//! and drives the new code path against real fixtures: a local bare-git
//! remote for the bundle and an `httpmock` server for the documents API.

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use git2::{build::CheckoutBuilder, Repository};
use httpmock::prelude::*;
use hydra::client::{HydraClient, HydraClientInterface};
use hydra::command::sessions::mounts::orchestrator::run_phase;
use hydra::command::sessions::mounts::spec::{
    find_documents_dir, instantiate, InstantiateInputs, MountSpecError,
};
use hydra::git::{commit_changes, configure_repo, push_branch, stage_all_changes};
use hydra_common::documents::ListDocumentsResponse;
use hydra_common::sessions::{Bundle, MountItem, MountSpec, RelativePath};
use hydra_common::{
    BuildCacheContext, BuildCacheSettings, BuildCacheStorageConfig, RepoName, SessionId,
};
use reqwest::Client as HttpClient;
use tempfile::TempDir;

const TEST_HYDRA_TOKEN: &str = "test-hydra-token";

fn rel(s: &str) -> RelativePath {
    RelativePath::new(s).expect("relative path")
}

fn task_session_id() -> SessionId {
    SessionId::from_str("s-spectest").expect("test session id")
}

fn mock_client(server: &MockServer) -> Arc<dyn HydraClientInterface> {
    let client =
        HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())
            .expect("build mock client");
    Arc::new(client)
}

fn promote_branch_to_main(repo: &Repository) -> Result<()> {
    let head_commit = repo
        .head()
        .context("resolve HEAD for upstream repo")?
        .peel_to_commit()
        .context("peel HEAD commit for upstream repo")?;
    repo.branch("main", &head_commit, true)
        .context("create 'main' branch in upstream repo")?;
    repo.set_head("refs/heads/main")
        .context("set HEAD to 'main' in upstream repo")?;
    let mut checkout = CheckoutBuilder::new();
    checkout.safe();
    repo.checkout_head(Some(&mut checkout))
        .context("checkout 'main' in upstream repo")?;
    Ok(())
}

/// Local bare-git remote + upstream working tree, mirroring the pattern in
/// `hydra/src/command/sessions/mounts/bundle.rs::tests::RemoteFixture`.
struct GitRemote {
    _remote_dir: TempDir,
    _upstream_dir: TempDir,
    remote_path: String,
    remote_root: std::path::PathBuf,
}

impl GitRemote {
    fn new() -> Result<Self> {
        let remote_dir = tempfile::tempdir().context("create remote tempdir")?;
        Repository::init_bare(remote_dir.path()).context("init bare remote repo")?;

        let upstream_dir = tempfile::tempdir().context("create upstream tempdir")?;
        Repository::init(upstream_dir.path()).context("init upstream repo")?;
        configure_repo(upstream_dir.path(), "Test User", "test@example.com")?;
        std::fs::write(upstream_dir.path().join("README.md"), "initial content")
            .context("write initial README")?;
        stage_all_changes(upstream_dir.path())?;
        commit_changes(upstream_dir.path(), "initial commit")?;

        let remote_path = remote_dir
            .path()
            .to_str()
            .ok_or_else(|| anyhow!("remote path contains invalid UTF-8"))?
            .to_string();

        let upstream_repo = Repository::open(upstream_dir.path()).context("reopen upstream")?;
        upstream_repo
            .remote("origin", &remote_path)
            .context("add origin remote to upstream")?;
        promote_branch_to_main(&upstream_repo)?;
        push_branch(upstream_dir.path(), "main", None, false)
            .context("push main branch to remote fixture")?;
        let bare = Repository::open_bare(remote_dir.path()).context("reopen bare remote")?;
        bare.set_head("refs/heads/main")
            .context("set remote HEAD to main")?;

        let remote_root = remote_dir.path().to_path_buf();
        Ok(Self {
            _remote_dir: remote_dir,
            _upstream_dir: upstream_dir,
            remote_path,
            remote_root,
        })
    }
}

fn empty_documents_response() -> ListDocumentsResponse {
    ListDocumentsResponse::new(Vec::new())
}

/// A 3-item `MountSpec` (Bundle + BuildCache + Documents) drives `instantiate`
/// to set up the agent CWD at `dest/repo`, `HYDRA_DOCUMENTS_DIR` at
/// `dest/documents`, and runs `setup` then `save` for all three mounts in
/// order against real fixtures.
#[tokio::test]
async fn three_item_mount_spec_runs_full_lifecycle() -> Result<()> {
    let docs_server = MockServer::start();
    let list_mock = docs_server.mock(|when, then| {
        when.method(GET).path("/v1/documents");
        then.status(200).json_body_obj(&empty_documents_response());
    });
    let client = mock_client(&docs_server);

    let remote = GitRemote::new()?;
    let dest = tempfile::tempdir().context("dest tempdir")?;
    let cache_root = tempfile::tempdir().context("cache root tempdir")?;
    let worker_home_dir = tempfile::tempdir().context("worker home tempdir")?;

    let session_id = task_session_id();
    let spec = MountSpec::new(
        rel("repo"),
        vec![
            MountItem::Bundle {
                target: rel("repo"),
                bundle: Bundle::GitRepository {
                    url: remote.remote_path.clone(),
                    rev: "main".to_string(),
                },
            },
            MountItem::BuildCache {
                repo_target: rel("repo"),
                service_repo_name: RepoName::new("acme", "widgets").expect("repo name"),
                context: BuildCacheContext {
                    storage: BuildCacheStorageConfig::FileSystem {
                        root_dir: cache_root.path().to_string_lossy().into_owned(),
                    },
                    settings: BuildCacheSettings::default(),
                },
            },
            MountItem::Documents {
                target: rel("documents"),
            },
        ],
    );

    let docs_target = find_documents_dir(&spec).expect("Documents item present");
    assert_eq!(docs_target.as_path(), Path::new("documents"));

    let result = instantiate(
        &spec,
        InstantiateInputs {
            github_token: None,
            worker_home_dir: Some(worker_home_dir.path().to_path_buf()),
            dest: dest.path(),
            client: Arc::clone(&client),
            session_id: session_id.clone(),
            issue_branch_id: None,
        },
    )
    .expect("instantiate");

    assert_eq!(result.working_dir, dest.path().join("repo"));
    assert_eq!(result.mounts.len(), 3);

    let mut mounts = result.mounts;
    let mut errors: Vec<anyhow::Error> = Vec::new();
    for mount in mounts.iter_mut() {
        run_phase(mount.setup_phase(), || mount.setup(), &mut errors)
            .await
            .expect("setup phase");
    }
    assert!(
        errors.is_empty(),
        "setup phase errors must be empty: {errors:?}"
    );

    // Bundle mount set up the clone.
    assert!(
        dest.path().join("repo").join("README.md").is_file(),
        "BundleMount setup should have cloned the repo into dest/repo"
    );
    // Documents mount created its directory and called list_documents once.
    assert!(
        dest.path().join("documents").is_dir(),
        "DocumentsMount setup should have created dest/documents"
    );

    // Simulate agent work: create a new file under the repo.
    std::fs::write(
        dest.path().join("repo").join("AGENT_WORK.md"),
        "edits from the agent\n",
    )?;

    for mount in mounts.iter_mut() {
        let Some(phase) = mount.save_phase() else {
            continue;
        };
        run_phase(phase, || mount.save(), &mut errors)
            .await
            .expect("save phase");
    }
    assert!(
        errors.is_empty(),
        "save phase errors must be empty: {errors:?}"
    );

    // Bundle save pushed `hydra/<session>/head` with the agent's commit.
    let task_head = format!("refs/heads/hydra/{session_id}/head");
    let remote_repo = Repository::open(&remote.remote_root).context("open remote repo")?;
    let pushed = remote_repo
        .find_reference(&task_head)
        .with_context(|| format!("find pushed reference {task_head}"))?;
    let pushed_commit = pushed.peel_to_commit().context("peel pushed reference")?;
    let pushed_tree = pushed_commit.tree().context("peel pushed tree")?;
    assert!(
        pushed_tree.get_path(Path::new("AGENT_WORK.md")).is_ok(),
        "BundleMount save should have committed AGENT_WORK.md to the pushed task branch"
    );

    // Documents mount hit list_documents at least twice (setup + save).
    assert!(
        list_mock.hits() >= 2,
        "DocumentsMount setup + save should have hit list_documents at least twice (got {})",
        list_mock.hits()
    );

    Ok(())
}

/// `MountItem::Unknown` produces a fatal `MountSpecError::UnsupportedItem`
/// — the client refuses to silently skip a server-required mount.
#[tokio::test]
async fn unknown_mount_item_is_fatal() {
    let docs_server = MockServer::start();
    let client = mock_client(&docs_server);
    let dest = tempfile::tempdir().expect("dest tempdir");
    let session_id = task_session_id();

    let spec = MountSpec::new(
        rel("repo"),
        vec![
            MountItem::Bundle {
                target: rel("repo"),
                bundle: Bundle::None,
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
            dest: dest.path(),
            client,
            session_id,
            issue_branch_id: None,
        },
    );

    match result {
        Ok(_) => panic!("MountItem::Unknown must abort instantiate"),
        Err(err) => assert!(
            matches!(err, MountSpecError::UnsupportedItem),
            "expected UnsupportedItem, got {err:?}"
        ),
    }
}

/// A 2-item spec (no BuildCache) lays out `[Bundle::None, Documents]` and
/// still runs cleanly: the agent CWD lands at `dest/repo`, the documents
/// directory at `dest/documents`, and neither mount tries to do git work.
#[tokio::test]
async fn two_item_spec_with_bundle_none_runs_lifecycle() -> Result<()> {
    let docs_server = MockServer::start();
    let _list_mock = docs_server.mock(|when, then| {
        when.method(GET).path("/v1/documents");
        then.status(200).json_body_obj(&empty_documents_response());
    });
    let client = mock_client(&docs_server);

    let dest = tempfile::tempdir().context("dest tempdir")?;
    let session_id = task_session_id();
    let spec = MountSpec::new(
        rel("repo"),
        vec![
            MountItem::Bundle {
                target: rel("repo"),
                bundle: Bundle::None,
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
            dest: dest.path(),
            client,
            session_id,
            issue_branch_id: None,
        },
    )
    .expect("instantiate");

    assert_eq!(result.working_dir, dest.path().join("repo"));
    assert_eq!(result.mounts.len(), 2);

    let mut mounts = result.mounts;
    let mut errors: Vec<anyhow::Error> = Vec::new();
    for mount in mounts.iter_mut() {
        run_phase(mount.setup_phase(), || mount.setup(), &mut errors)
            .await
            .expect("setup phase");
    }
    assert!(errors.is_empty(), "setup errors: {errors:?}");
    assert!(dest.path().join("repo").is_dir());
    assert!(dest.path().join("documents").is_dir());

    // Bundle::None has no save phase; only Documents may run save.
    let save_phases: Vec<_> = mounts.iter().map(|m| m.save_phase().is_some()).collect();
    assert_eq!(save_phases, vec![false, true]);
    Ok(())
}
