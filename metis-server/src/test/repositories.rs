use crate::{
    app::{ServiceRepository, ServiceRepositoryConfig, ServiceState},
    test::{spawn_test_server_with_state, test_client, test_state},
};
use git2::{Repository, Signature};
use metis_common::{
    RepoName,
    repositories::{
        CreateRepositoryRequest, ListRepositoriesResponse, UpdateRepositoryRequest,
        UpsertRepositoryResponse,
    },
};
use reqwest::StatusCode;
use std::{collections::HashMap, path::Path, str::FromStr, sync::Arc};
use tempfile::TempDir;

#[tokio::test]
async fn list_repositories_returns_config_without_secrets() -> anyhow::Result<()> {
    let (name, repository) = crate::test::common::service_repository();
    let mut state = test_state();
    state.service_state = Arc::new(ServiceState::with_repositories(HashMap::from([(
        name.clone(),
        repository,
    )])));
    let server = spawn_test_server_with_state(state).await?;
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
    assert!(repository.github_token_present);
    assert!(repository.default_branch.is_some());
    assert!(repository.default_image.is_some());

    Ok(())
}

#[tokio::test]
async fn create_repository_initializes_cache_and_merge_queue() -> anyhow::Result<()> {
    let state = test_state();
    let service_state = state.service_state.clone();
    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();

    let remote_dir = create_remote_repository()?;
    let remote_url = repo_url(&remote_dir);
    let name = RepoName::from_str("dourolabs/new-repo")?;

    let payload = CreateRepositoryRequest {
        name: name.clone(),
        repository: ServiceRepositoryConfig {
            remote_url: remote_url.clone(),
            default_branch: Some("main".to_string()),
            github_token: Some("token-456".to_string()),
            default_image: Some("ghcr.io/example/new-repo:main".to_string()),
        },
    };

    let response = client
        .post(format!("{}/v1/repositories", server.base_url()))
        .json(&payload)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let body: UpsertRepositoryResponse = response.json().await?;
    assert_eq!(body.repository.name, name);
    assert!(body.repository.github_token_present);
    assert_eq!(body.repository.remote_url, remote_url);
    assert_eq!(body.repository.default_branch.as_deref(), Some("main"));
    assert_eq!(
        body.repository.default_image.as_deref(),
        Some("ghcr.io/example/new-repo:main")
    );

    let stored = service_state
        .repository(&name)
        .expect("repository should be stored");
    assert_eq!(stored.remote_url, remote_url);
    assert_eq!(stored.github_token.as_deref(), Some("token-456"));

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

    let repository = ServiceRepository {
        name: name.clone(),
        remote_url: repo_url(&original_remote),
        default_branch: Some("develop".to_string()),
        github_token: Some("token-123".to_string()),
        default_image: Some("ghcr.io/example/repo:main".to_string()),
    };
    let mut state = test_state();
    state.service_state = Arc::new(ServiceState::with_repositories(HashMap::from([(
        name.clone(),
        repository,
    )])));
    let service_state = state.service_state.clone();
    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();

    let payload = UpdateRepositoryRequest {
        repository: ServiceRepositoryConfig {
            remote_url: repo_url(&updated_remote),
            default_branch: None,
            github_token: None,
            default_image: None,
        },
    };

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
    assert_eq!(body.repository.remote_url, repo_url(&updated_remote));
    assert!(!body.repository.github_token_present);
    assert!(body.repository.default_branch.is_none());
    assert!(body.repository.default_image.is_none());

    let stored = service_state
        .repository(&name)
        .expect("repository should be stored");
    assert_eq!(stored.remote_url, repo_url(&updated_remote));
    assert!(stored.github_token.is_none());
    assert!(stored.default_branch.is_none());
    assert!(stored.default_image.is_none());

    let merge_queues = service_state.merge_queues.read().await;
    let repo_queues = merge_queues
        .get(&name)
        .expect("merge queues should exist after update");
    assert!(repo_queues.is_empty());

    let cache_paths = service_state.cached_repository_paths().await;
    assert!(cache_paths.contains_key(&name));

    Ok(())
}

#[tokio::test]
async fn update_unknown_repository_returns_not_found() -> anyhow::Result<()> {
    let state = test_state();
    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();
    let remote_dir = create_remote_repository()?;

    let payload = UpdateRepositoryRequest {
        repository: ServiceRepositoryConfig {
            remote_url: repo_url(&remote_dir),
            default_branch: None,
            github_token: None,
            default_image: None,
        },
    };

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
    let mut state = test_state();
    state.service_state = Arc::new(ServiceState::with_repositories(HashMap::from([(
        name.clone(),
        repository,
    )])));
    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();

    let bad_payload = CreateRepositoryRequest {
        name: RepoName::from_str("dourolabs/new-repo")?,
        repository: ServiceRepositoryConfig {
            remote_url: "   ".to_string(),
            default_branch: None,
            github_token: None,
            default_image: None,
        },
    };
    let bad_response = client
        .post(format!("{}/v1/repositories", server.base_url()))
        .json(&bad_payload)
        .send()
        .await?;
    assert_eq!(bad_response.status(), StatusCode::BAD_REQUEST);

    let duplicate_payload = CreateRepositoryRequest {
        name: name.clone(),
        repository: ServiceRepositoryConfig {
            remote_url: "https://example.com/new-repo.git".to_string(),
            default_branch: None,
            github_token: None,
            default_image: None,
        },
    };
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
    let repository = Repository::init(directory.path())?;
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
    repository: &Repository,
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
