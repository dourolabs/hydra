use crate::{
    app::{GitRepository, ServiceState},
    test::{spawn_test_server_with_state, test_client, test_state},
};
use anyhow::Result;
use git2::{Oid, Repository, ResetType, Signature};
use metis_common::{
    PatchId, RepoName,
    merge_queues::{EnqueueMergePatchRequest, MergeQueue},
    patches::{GitOid, Patch, PatchCommitRange, PatchStatus},
};
use reqwest::StatusCode;
use std::{collections::HashMap, fs, path::Path, str::FromStr, sync::Arc};
use tempfile::TempDir;

fn app_state_with_repo(repo_name: RepoName, remote_path: String) -> crate::app::AppState {
    let mut state = test_state();
    let repository = GitRepository {
        remote_url: remote_path,
        default_branch: Some("main".to_string()),
        github_token: None,
        default_image: None,
    };
    state.service_state = Arc::new(ServiceState::with_repositories(HashMap::from([(
        repo_name, repository,
    )])));
    state
}

fn init_service_repo(repo_name: &str) -> Result<(TempDir, Repository, RepoName, Oid)> {
    let repo_name = RepoName::from_str(repo_name)?;
    let tempdir = TempDir::new()?;
    let repo = Repository::init(tempdir.path())?;
    let base_commit = initial_commit(&repo, "README.md", "base\n", "base commit")?;
    {
        let base = repo.find_commit(base_commit)?;
        repo.branch("main", &base, true)?;
    }
    repo.set_head("refs/heads/main")?;

    Ok((tempdir, repo, repo_name, base_commit))
}

#[tokio::test]
async fn get_merge_queue_returns_empty_for_new_branch() -> Result<()> {
    let (tempdir, _repo, repo_name, _) = init_service_repo("dourolabs/api")?;
    let state = app_state_with_repo(repo_name, tempdir.path().to_str().unwrap().to_string());
    let server = spawn_test_server_with_state(state).await?;
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
async fn enqueue_patch_appends_to_queue() -> Result<()> {
    let (tempdir, repo, repo_name, base_commit) = init_service_repo("dourolabs/api")?;
    let patch_commit = commit_with_parent(
        &repo,
        base_commit,
        "README.md",
        "patched base\n",
        "feature patch",
    )?;
    repo.branch(
        "feature",
        &repo
            .find_commit(patch_commit)
            .expect("patch commit should exist"),
        true,
    )?;
    let state = app_state_with_repo(
        repo_name.clone(),
        tempdir.path().to_str().unwrap().to_string(),
    );
    let patch_id = {
        let mut store = state.store.write().await;
        store
            .add_patch(Patch {
                title: "feature patch".to_string(),
                description: "test patch".to_string(),
                commit_range: PatchCommitRange {
                    base: GitOid::from(base_commit),
                    head: GitOid::from(patch_commit),
                },
                status: PatchStatus::Open,
                is_automatic_backup: false,
                reviews: Vec::new(),
                service_repo_name: repo_name.clone(),
                github: None,
            })
            .await?
    };
    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();

    let response = client
        .post(format!(
            "{}/v1/merge-queues/dourolabs/api/main/patches",
            server.base_url()
        ))
        .json(&EnqueueMergePatchRequest {
            patch_id: patch_id.clone(),
        })
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
async fn merge_queue_requires_known_repository() -> Result<()> {
    let server = spawn_test_server_with_state(test_state()).await?;
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
        .json(&EnqueueMergePatchRequest {
            patch_id: PatchId::new(),
        })
        .send()
        .await?;

    assert_eq!(enqueue_response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

fn initial_commit(repo: &Repository, path: &str, contents: &str, message: &str) -> Result<Oid> {
    write_file(repo, path, contents)?;
    let mut index = repo.index()?;
    index.add_path(Path::new(path))?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let signature = Signature::now("tester", "tester@example.com")?;

    repo.commit(Some("HEAD"), &signature, &signature, message, &tree, &[])
        .map_err(Into::into)
}

fn commit_with_parent(
    repo: &Repository,
    parent: Oid,
    path: &str,
    contents: &str,
    message: &str,
) -> Result<Oid> {
    repo.reset(&repo.find_object(parent, None)?, ResetType::Hard, None)?;
    write_file(repo, path, contents)?;
    let mut index = repo.index()?;
    index.add_path(Path::new(path))?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let signature = Signature::now("tester", "tester@example.com")?;
    let parent_commit = repo.find_commit(parent)?;

    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        message,
        &tree,
        &[&parent_commit],
    )
    .map_err(Into::into)
}

fn write_file(repo: &Repository, path: &str, contents: &str) -> Result<()> {
    let workdir = repo
        .workdir()
        .ok_or_else(|| anyhow::anyhow!("repository missing working directory"))?;
    let full_path = workdir.join(path);
    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(full_path, contents)?;
    Ok(())
}
