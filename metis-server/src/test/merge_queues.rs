use crate::{
    app::Repository as RepositoryConfig,
    domain::{actors::ActorRef, patches::{Patch, PatchStatus}},
    test::{TestStateHandles, spawn_test_server_with_state, test_client, test_state_handles},
};
use git2::{Repository as GitRepository, Signature, build::CheckoutBuilder};
use metis_common::{
    PatchId, RepoName,
    merge_queues::{EnqueueMergePatchRequest, MergeQueue},
};
use reqwest::StatusCode;
use std::{path::Path, str::FromStr};
use tempfile::TempDir;

async fn state_with_repo(repo_name: &str) -> anyhow::Result<(TestStateHandles, TempDir)> {
    let repo = RepoName::from_str(repo_name).expect("repo name should be valid");
    let remote_dir = TempDir::new()?;
    let repository = GitRepository::init(remote_dir.path())?;
    let signature = Signature::now("Tester", "tester@example.com")?;
    commit_file(&repository, "README.md", "base\n", "base", &signature)?;

    let handles = test_state_handles();
    handles
        .state
        .create_repository(
            repo.clone(),
            RepositoryConfig::new(
                remote_dir
                    .path()
                    .to_str()
                    .expect("tempdir path should be utf-8")
                    .to_string(),
                None,
                None,
            ),
            ActorRef::test(),
        )
        .await?;

    Ok((handles, remote_dir))
}

async fn state_with_repo_and_patch(
    repo_name: &str,
) -> anyhow::Result<(TestStateHandles, PatchId, TempDir)> {
    let repo = RepoName::from_str(repo_name)?;
    let (remote_dir, diff) = create_repository_with_patch()?;

    let handles = test_state_handles();
    handles
        .state
        .create_repository(
            repo.clone(),
            RepositoryConfig::new(
                remote_dir
                    .path()
                    .to_str()
                    .expect("tempdir path is valid utf-8")
                    .to_string(),
                Some("main".to_string()),
                None,
            ),
            ActorRef::test(),
        )
        .await?;

    let patch = Patch::new(
        "Test patch".to_string(),
        "Patch for merge queue enqueue test".to_string(),
        diff,
        PatchStatus::Open,
        false,
        None,
        Vec::new(),
        repo.clone(),
        None,
    );

    let (patch_id, _) = handles.store.add_patch(patch).await?;

    Ok((handles, patch_id, remote_dir))
}

fn create_repository_with_patch() -> anyhow::Result<(TempDir, String)> {
    let remote_dir = TempDir::new()?;
    let repository = GitRepository::init(remote_dir.path())?;
    let signature = Signature::now("Tester", "tester@example.com")?;

    let base_commit = commit_file(&repository, "README.md", "base\n", "base", &signature)?;
    repository.branch("main", &repository.find_commit(base_commit)?, true)?;
    repository.set_head("refs/heads/main")?;
    repository.checkout_head(Some(CheckoutBuilder::new().force()))?;

    repository.branch("feature", &repository.find_commit(base_commit)?, true)?;
    repository.set_head("refs/heads/feature")?;
    repository.checkout_head(Some(CheckoutBuilder::new().force()))?;
    let patch_commit = commit_file(
        &repository,
        "feature.txt",
        "change\n",
        "feature",
        &signature,
    )?;

    repository.set_head("refs/heads/main")?;
    repository.checkout_head(Some(CheckoutBuilder::new().force()))?;

    let diff = git_diff_for_commits(remote_dir.path(), base_commit, patch_commit)?;

    Ok((remote_dir, diff))
}

fn commit_file(
    repo: &GitRepository,
    name: &str,
    contents: &str,
    message: &str,
    signature: &Signature<'_>,
) -> anyhow::Result<git2::Oid> {
    let workdir = repo
        .workdir()
        .expect("repository should be a working tree")
        .to_path_buf();
    let full_path = workdir.join(Path::new(name));
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&full_path, contents)?;

    let mut index = repo.index()?;
    index.add_path(Path::new(name))?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let head_commit = repo.head().ok().and_then(|reference| {
        reference
            .target()
            .and_then(|target| repo.find_commit(target).ok())
    });
    let parents: Vec<&git2::Commit> = head_commit.iter().collect();

    let commit_id = repo.commit(Some("HEAD"), signature, signature, message, &tree, &parents)?;

    Ok(commit_id)
}

fn git_diff_for_commits(
    repo_path: &Path,
    base: git2::Oid,
    head: git2::Oid,
) -> anyhow::Result<String> {
    let repo_str = repo_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("invalid repo path"))?;
    let output = std::process::Command::new("git")
        .args([
            "-C",
            repo_str,
            "diff",
            "--no-ext-diff",
            "--no-color",
            &format!("{base}..{head}"),
        ])
        .output()?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }

    anyhow::bail!(
        "git diff failed: {}",
        String::from_utf8_lossy(&output.stderr)
    )
}

#[tokio::test]
async fn get_merge_queue_returns_empty_for_new_branch() -> anyhow::Result<()> {
    let (handles, _remote_dir) = state_with_repo("dourolabs/api").await?;
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let response = client
        .get(format!(
            "{}/v1/merge-queues/dourolabs/api/main/patches",
            server.base_url()
        ))
        .send()
        .await?;

    assert!(response.status().is_success());
    let queue: MergeQueue = response.json().await?;
    assert!(queue.patches.is_empty());

    Ok(())
}

#[tokio::test]
async fn enqueue_patch_appends_to_queue() -> anyhow::Result<()> {
    let (handles, patch_id, _remote_dir) = state_with_repo_and_patch("dourolabs/api").await?;
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let response = client
        .post(format!(
            "{}/v1/merge-queues/dourolabs/api/main/patches",
            server.base_url()
        ))
        .json(&EnqueueMergePatchRequest::new(patch_id.clone()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let queue: MergeQueue = response.json().await?;
    assert_eq!(queue.patches, vec![patch_id.clone()]);

    let fetch_response = client
        .get(format!(
            "{}/v1/merge-queues/dourolabs/api/main/patches",
            server.base_url()
        ))
        .send()
        .await?;

    assert!(fetch_response.status().is_success());
    let fetched_queue: MergeQueue = fetch_response.json().await?;
    assert_eq!(fetched_queue.patches, vec![patch_id]);

    Ok(())
}

#[tokio::test]
async fn merge_queue_requires_known_repository() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let response = client
        .get(format!(
            "{}/v1/merge-queues/unknown/unknown/main/patches",
            server.base_url()
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let enqueue_response = client
        .post(format!(
            "{}/v1/merge-queues/unknown/unknown/main/patches",
            server.base_url()
        ))
        .json(&EnqueueMergePatchRequest::new(PatchId::new()))
        .send()
        .await?;

    assert_eq!(enqueue_response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}
