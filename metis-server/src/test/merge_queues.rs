use crate::{
    app::{GitRepository, ServiceState},
    test::{spawn_test_server_with_state, test_client, test_state},
};
use metis_common::{
    PatchId, RepoName,
    merge_queues::{EnqueueMergePatchRequest, MergeQueue},
};
use reqwest::StatusCode;
use std::{collections::HashMap, str::FromStr, sync::Arc};

fn state_with_repo(repo_name: &str) -> crate::app::AppState {
    let repo = RepoName::from_str(repo_name).expect("repo name should be valid");
    let mut state = test_state();
    let repository = GitRepository {
        remote_url: format!("https://example.com/{}.git", repo.as_str()),
        default_branch: None,
        github_token: None,
        default_image: None,
    };
    state.service_state = Arc::new(ServiceState::with_repositories(HashMap::from([(
        repo, repository,
    )])));

    state
}

#[tokio::test]
async fn get_merge_queue_returns_empty_for_new_branch() -> anyhow::Result<()> {
    let state = state_with_repo("dourolabs/api");
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
async fn enqueue_patch_appends_to_queue() -> anyhow::Result<()> {
    let state = state_with_repo("dourolabs/api");
    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();
    let patch_id = PatchId::new();

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
async fn merge_queue_requires_known_repository() -> anyhow::Result<()> {
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
