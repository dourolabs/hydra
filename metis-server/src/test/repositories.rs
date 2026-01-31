use crate::{
    app::Repository,
    test::{spawn_test_server_with_state, test_client, test_state_handles},
};
use git2::{Repository as GitRepository, Signature};
use metis_common::{
    RepoName,
    repositories::{
        CreateRepositoryRequest, ListRepositoriesResponse, UpdateRepositoryRequest,
        UpsertRepositoryResponse,
    },
};
use reqwest::StatusCode;
use std::{path::Path, str::FromStr};
use tempfile::TempDir;

#[tokio::test]
async fn list_repositories_returns_config_without_secrets() -> anyhow::Result<()> {
    let (name, repository) = crate::test::common::service_repository();
    let handles = test_state_handles();
    handles
        .state
        .create_repository(name.clone(), repository.clone())
        .await?;
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/repositories", server.base_url()))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let body: ListRepositoriesResponse = response.json().await?;
    assert_eq!(body.repositories.len(), 1);
    let repository = &body.repositories[0];

    assert_eq!(repository.name, name);
    assert!(repository.repository.default_branch.is_some());
    assert!(repository.repository.default_image.is_some());
    assert!(repository.repository.content_summary.is_none());

    Ok(())
}

#[tokio::test]
async fn create_repository_initializes_cache_and_merge_queue() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let service_state = handles.state.service_state.clone();
    let check_state = handles.state.clone();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let remote_dir = create_remote_repository()?;
    let remote_url = repo_url(&remote_dir);
    let name = RepoName::from_str("dourolabs/new-repo")?;
    let summary = Some("## Summary\n\n- bullet one\n- bullet two".to_string());

    let payload = CreateRepositoryRequest::new(
        name.clone(),
        Repository::new(
            remote_url.clone(),
            Some("main".to_string()),
            Some("ghcr.io/example/new-repo:main".to_string()),
            summary.clone(),
        ),
    );

    let response = client
        .post(format!("{}/v1/repositories", server.base_url()))
        .json(&payload)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let body: UpsertRepositoryResponse = response.json().await?;
    assert_eq!(body.repository.name, name);
    assert_eq!(body.repository.repository.remote_url, remote_url);
    assert_eq!(
        body.repository.repository.default_branch.as_deref(),
        Some("main")
    );
    assert_eq!(
        body.repository.repository.default_image.as_deref(),
        Some("ghcr.io/example/new-repo:main")
    );
    assert_eq!(
        body.repository.repository.content_summary.as_deref(),
        summary.as_deref()
    );

    let stored = check_state.repository_from_store(&name).await?;
    assert_eq!(stored.remote_url, remote_url);
    assert_eq!(stored.content_summary, summary);

    let merge_queues = service_state.merge_queues.read().await;
    assert!(!merge_queues.contains_key(&name));
    drop(merge_queues);

    let cache_paths = service_state.cached_repository_paths().await;
    assert!(!cache_paths.contains_key(&name));

    drop(server);
    service_state.ensure_cached(&name, &stored).await?;

    let merge_queues = service_state.merge_queues.read().await;
    assert!(merge_queues.contains_key(&name));

    let cache_paths = service_state.cached_repository_paths().await;
    assert!(cache_paths.contains_key(&name));

    Ok(())
}

#[tokio::test]
async fn update_repository_replaces_config_and_clears_optionals() -> anyhow::Result<()> {
    let name = RepoName::from_str("dourolabs/metis")?;
    let original_remote = create_remote_repository()?;
    let updated_remote = create_remote_repository()?;

    let handles = test_state_handles();
    let check_state = handles.state.clone();
    let service_state = handles.state.service_state.clone();
    let repository = Repository::new(
        repo_url(&original_remote),
        Some("develop".to_string()),
        Some("ghcr.io/example/repo:main".to_string()),
        Some("Initial summary".to_string()),
    );
    handles
        .state
        .create_repository(name.clone(), repository.clone())
        .await?;
    service_state.ensure_cached(&name, &repository).await?;
    let service_state = handles.state.service_state.clone();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let payload =
        UpdateRepositoryRequest::new(Repository::new(repo_url(&updated_remote), None, None, None));

    let response = client
        .put(format!(
            "{}/v1/repositories/{}/{}",
            server.base_url(),
            name.organization,
            name.repo
        ))
        .json(&payload)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let body: UpsertRepositoryResponse = response.json().await?;
    assert_eq!(body.repository.name, name);
    assert_eq!(
        body.repository.repository.remote_url,
        repo_url(&updated_remote)
    );
    assert!(body.repository.repository.default_branch.is_none());
    assert!(body.repository.repository.default_image.is_none());
    assert!(body.repository.repository.content_summary.is_none());

    let stored = check_state.repository_from_store(&name).await?;
    assert_eq!(stored.remote_url, repo_url(&updated_remote));
    assert!(stored.default_branch.is_none());
    assert!(stored.default_image.is_none());
    assert!(stored.content_summary.is_none());

    let merge_queues = service_state.merge_queues.read().await;
    assert!(!merge_queues.contains_key(&name));

    let cache_paths = service_state.cached_repository_paths().await;
    assert!(!cache_paths.contains_key(&name));

    Ok(())
}

#[tokio::test]
async fn update_unknown_repository_returns_not_found() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();
    let remote_dir = create_remote_repository()?;

    let payload =
        UpdateRepositoryRequest::new(Repository::new(repo_url(&remote_dir), None, None, None));

    let response = client
        .put(format!(
            "{}/v1/repositories/{}/{}",
            server.base_url(),
            "dourolabs",
            "missing"
        ))
        .json(&payload)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn create_repository_rejects_empty_remote_and_duplicate_name() -> anyhow::Result<()> {
    let (name, repository) = crate::test::common::service_repository();
    let handles = test_state_handles();
    handles
        .state
        .create_repository(
            name.clone(),
            Repository::new(
                repository.remote_url.clone(),
                repository.default_branch.clone(),
                repository.default_image.clone(),
                repository.content_summary.clone(),
            ),
        )
        .await?;
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let bad_payload = CreateRepositoryRequest::new(
        RepoName::from_str("dourolabs/new-repo")?,
        Repository::new("   ".to_string(), None, None, None),
    );
    let bad_response = client
        .post(format!("{}/v1/repositories", server.base_url()))
        .json(&bad_payload)
        .send()
        .await?;
    assert_eq!(bad_response.status(), StatusCode::BAD_REQUEST);

    let duplicate_payload = CreateRepositoryRequest::new(
        name.clone(),
        Repository::new(
            "https://example.com/new-repo.git".to_string(),
            None,
            None,
            None,
        ),
    );
    let duplicate_response = client
        .post(format!("{}/v1/repositories", server.base_url()))
        .json(&duplicate_payload)
        .send()
        .await?;
    assert_eq!(duplicate_response.status(), StatusCode::CONFLICT);

    Ok(())
}

fn create_remote_repository() -> anyhow::Result<TempDir> {
    let directory = TempDir::new()?;
    let repository = GitRepository::init(directory.path())?;
    let signature = Signature::now("Tester", "tester@example.com")?;

    commit_file(
        &repository,
        "README.md",
        "hello\n",
        "initial commit",
        &signature,
    )?;

    Ok(directory)
}

fn commit_file(
    repository: &GitRepository,
    path: &str,
    contents: &str,
    message: &str,
    signature: &Signature<'_>,
) -> anyhow::Result<git2::Oid> {
    let workdir = repository
        .workdir()
        .expect("repository should have a working tree")
        .to_path_buf();
    let full_path = workdir.join(Path::new(path));
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&full_path, contents)?;

    let mut index = repository.index()?;
    index.add_path(Path::new(path))?;
    let tree_id = index.write_tree()?;
    let tree = repository.find_tree(tree_id)?;

    let head_commit = repository.head().ok().and_then(|reference| {
        reference
            .target()
            .and_then(|target| repository.find_commit(target).ok())
    });

    let parents: Vec<&git2::Commit> = head_commit.iter().collect();
    let commit_id =
        repository.commit(Some("HEAD"), signature, signature, message, &tree, &parents)?;

    Ok(commit_id)
}

fn repo_url(dir: &TempDir) -> String {
    dir.path()
        .to_str()
        .expect("tempdir path should be utf-8")
        .to_string()
}
