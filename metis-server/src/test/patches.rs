use super::common::{default_image, patch_diff, service_repo_name};
use crate::{
    domain::{
        issues::{
            Issue, IssueRecord, IssueStatus, IssueType, UpsertIssueRequest, UpsertIssueResponse,
        },
        jobs::BundleSpec,
        patches::{
            GithubPr, ListPatchesResponse, Patch, PatchRecord, PatchStatus, SearchPatchesQuery,
            UpsertPatchRequest, UpsertPatchResponse,
        },
        users::{User, Username},
    },
    store::{Status, Task},
    test_utils::{
        spawn_test_server, spawn_test_server_with_state, test_client, test_state_handles,
        test_state_with_imgur_api_base_url,
    },
};
use chrono::Utc;
use httpmock::prelude::HttpMockRequest;
use httpmock::{Method::POST, MockServer};
use metis_common::{
    PatchId,
    api::v1::patches::{CreatePatchAssetResponse, ListPatchVersionsResponse, PatchVersionRecord},
};
use reqwest::Client;
use reqwest::StatusCode;
use serde_json::json;
use std::collections::HashMap;

#[tokio::test]
async fn patches_can_be_created_and_retrieved() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let patch = Patch::new(
        "Initial patch".to_string(),
        "initial patch".to_string(),
        patch_diff(),
        PatchStatus::Open,
        false,
        None,
        Vec::new(),
        service_repo_name(),
        None,
    );

    let response = client
        .post(format!("{}/v1/patches", server.base_url()))
        .json(&UpsertPatchRequest::new(patch.clone()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let created: UpsertPatchResponse = response.json().await?;
    assert!(!created.patch_id.as_ref().is_empty());

    let fetched: PatchRecord = client
        .get(format!(
            "{}/v1/patches/{}",
            server.base_url(),
            created.patch_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(fetched.id, created.patch_id);
    assert_eq!(fetched.patch, patch);
    Ok(())
}

#[tokio::test]
async fn patch_versions_endpoints_return_history() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let patch = Patch::new(
        "Initial patch".to_string(),
        "initial patch".to_string(),
        patch_diff(),
        PatchStatus::Open,
        false,
        None,
        Vec::new(),
        service_repo_name(),
        None,
    );

    let response = client
        .post(format!("{}/v1/patches", server.base_url()))
        .json(&UpsertPatchRequest::new(patch.clone()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let created: UpsertPatchResponse = response.json().await?;

    let updated_patch = Patch::new(
        "Updated patch".to_string(),
        "updated patch".to_string(),
        patch_diff(),
        PatchStatus::Open,
        false,
        None,
        Vec::new(),
        service_repo_name(),
        None,
    );
    let _updated: UpsertPatchResponse = client
        .put(format!(
            "{}/v1/patches/{}",
            server.base_url(),
            created.patch_id
        ))
        .json(&UpsertPatchRequest::new(updated_patch))
        .send()
        .await?
        .json()
        .await?;

    let versions: ListPatchVersionsResponse = client
        .get(format!(
            "{}/v1/patches/{}/versions",
            server.base_url(),
            created.patch_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(versions.versions.len(), 2);
    assert_eq!(versions.versions[0].patch_id, created.patch_id);
    assert_eq!(versions.versions[0].version, 1);
    assert_eq!(versions.versions[0].patch.title, "Initial patch");
    assert_eq!(versions.versions[1].patch_id, created.patch_id);
    assert_eq!(versions.versions[1].version, 2);
    assert_eq!(versions.versions[1].patch.title, "Updated patch");

    let version: PatchVersionRecord = client
        .get(format!(
            "{}/v1/patches/{}/versions/2",
            server.base_url(),
            created.patch_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(version.version, 2);
    assert_eq!(version.patch_id, created.patch_id);
    assert_eq!(version.patch.title, "Updated patch");

    Ok(())
}

#[tokio::test]
async fn patch_version_endpoints_return_404s() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let missing: PatchId = "p-missing".parse().expect("valid patch id");
    let response = client
        .get(format!(
            "{}/v1/patches/{}/versions",
            server.base_url(),
            missing
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let patch = Patch::new(
        "Initial patch".to_string(),
        "initial patch".to_string(),
        patch_diff(),
        PatchStatus::Open,
        false,
        None,
        Vec::new(),
        service_repo_name(),
        None,
    );
    let response = client
        .post(format!("{}/v1/patches", server.base_url()))
        .json(&UpsertPatchRequest::new(patch))
        .send()
        .await?;
    let created: UpsertPatchResponse = response.json().await?;

    let response = client
        .get(format!(
            "{}/v1/patches/{}/versions/99",
            server.base_url(),
            created.patch_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn creating_patch_with_created_by_links_job() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let default_image = default_image();
    let job_id = super::common::task_id("t-emit");
    let check_state = handles.state.clone();
    handles
        .store
        .add_task_with_id(
            job_id.clone(),
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                image: Some(default_image),
                model: None,
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                status: Status::Created,
                last_message: None,
                error: None,
            },
            Utc::now(),
        )
        .await?;
    handles.state.transition_task_to_pending(&job_id).await?;
    handles.state.transition_task_to_running(&job_id).await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();
    let response = client
        .post(format!("{}/v1/patches", server.base_url()))
        .json(&UpsertPatchRequest::new(Patch::new(
            "artifact with creator".to_string(),
            "artifact with creator".to_string(),
            patch_diff(),
            PatchStatus::Open,
            false,
            Some(job_id.clone()),
            Vec::new(),
            service_repo_name(),
            None,
        )))
        .send()
        .await?;

    assert!(response.status().is_success());
    let created: UpsertPatchResponse = response.json().await?;

    let patch = check_state.get_patch(&created.patch_id).await?;
    assert_eq!(patch.item.created_by, Some(job_id));
    Ok(())
}

#[tokio::test]
async fn closing_patch_closes_merge_request_issues() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let base_patch = Patch::new(
        "link patch to issue".to_string(),
        "issue-linked patch".to_string(),
        patch_diff(),
        PatchStatus::Open,
        false,
        None,
        Vec::new(),
        service_repo_name(),
        None,
    );

    let created_patch: UpsertPatchResponse = client
        .post(format!("{}/v1/patches", server.base_url()))
        .json(&UpsertPatchRequest::new(base_patch.clone()))
        .send()
        .await?
        .json()
        .await?;

    let merge_request_issue = Issue::new(
        IssueType::MergeRequest,
        "linked merge request".to_string(),
        Username::from("creator"),
        String::new(),
        IssueStatus::Open,
        None,
        None,
        Vec::new(),
        vec![],
        vec![created_patch.patch_id.clone()],
    );

    let created_issue: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest {
            issue: merge_request_issue,
            job_id: None,
        })
        .send()
        .await?
        .json()
        .await?;

    let mut merged_patch = base_patch.clone();
    merged_patch.status = PatchStatus::Merged;
    client
        .put(format!(
            "{}/v1/patches/{}",
            server.base_url(),
            created_patch.patch_id
        ))
        .json(&UpsertPatchRequest::new(merged_patch))
        .send()
        .await?
        .error_for_status()?;

    let fetched_issue: IssueRecord = client
        .get(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created_issue.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(fetched_issue.issue.status, IssueStatus::Closed);
    Ok(())
}

#[tokio::test]
async fn list_patches_supports_filters() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let patch = Patch::new(
        "refactor logging".to_string(),
        "refactor logging".to_string(),
        patch_diff(),
        PatchStatus::Open,
        false,
        None,
        Vec::new(),
        service_repo_name(),
        None,
    );
    let filtered_patch = Patch::new(
        "login retry patch".to_string(),
        "login retry patch".to_string(),
        patch_diff(),
        PatchStatus::Open,
        false,
        None,
        Vec::new(),
        service_repo_name(),
        None,
    );

    for patch in [patch.clone(), filtered_patch.clone()] {
        let response = client
            .post(format!("{}/v1/patches", server.base_url()))
            .json(&UpsertPatchRequest::new(patch))
            .send()
            .await?;
        assert!(response.status().is_success());
    }

    let patch_results: ListPatchesResponse = client
        .get(format!("{}/v1/patches", server.base_url()))
        .query(&SearchPatchesQuery::new(Some("login".to_string())))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(patch_results.patches.len(), 1);
    assert_eq!(patch_results.patches[0].patch, filtered_patch);
    Ok(())
}

#[tokio::test]
async fn create_patch_asset_uploads_to_imgur() -> anyhow::Result<()> {
    let imgur_server = MockServer::start_async().await;

    let upload_mock = imgur_server.mock(|when, then| {
        when.method(POST)
            .path("/3/image")
            .header("authorization", "Client-ID imgur-client-id")
            .matches(|req: &HttpMockRequest| {
                let body = match req.body.as_ref() {
                    Some(body) => body,
                    None => return false,
                };
                let body_str = String::from_utf8_lossy(body);
                body_str.contains("binary-payload")
                    && body_str.contains("filename=\"screenshot.png\"")
            });
        then.status(200).json_body(json!({
            "data": { "link": "https://i.imgur.com/asset.png" },
            "success": true,
            "status": 200
        }));
    });

    let handles = test_state_with_imgur_api_base_url(imgur_server.base_url());
    let username = Username::from("octo");
    handles
        .store
        .add_user(User::new(
            username.clone(),
            42,
            "gh-token".to_string(),
            "gh-refresh".to_string(),
        ))
        .await?;
    let (actor, auth_token) = crate::domain::actors::Actor::new_for_user(username);
    handles.store.add_actor(actor).await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = client_with_token(auth_token);

    let patch = Patch::new(
        "asset patch".to_string(),
        "asset patch".to_string(),
        patch_diff(),
        PatchStatus::Open,
        false,
        None,
        Vec::new(),
        service_repo_name(),
        Some(GithubPr::new(
            "octo".to_string(),
            "repo".to_string(),
            42,
            None,
            None,
            None,
            None,
        )),
    );

    let created: UpsertPatchResponse = client
        .post(format!("{}/v1/patches", server.base_url()))
        .json(&UpsertPatchRequest::new(patch))
        .send()
        .await?
        .json()
        .await?;

    let response: CreatePatchAssetResponse = client
        .post(format!(
            "{}/v1/patches/{}/assets?name=screenshot.png",
            server.base_url(),
            created.patch_id
        ))
        .header("content-type", "image/png")
        .body("binary-payload")
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(response.asset_url, "https://i.imgur.com/asset.png");
    upload_mock.assert_hits(1);
    Ok(())
}

#[tokio::test]
async fn create_patch_asset_surfaces_imgur_400() -> anyhow::Result<()> {
    let imgur_server = MockServer::start_async().await;

    let upload_mock = imgur_server.mock(|when, then| {
        when.method(POST)
            .path("/3/image")
            .header("authorization", "Client-ID imgur-client-id");
        then.status(400).json_body(json!({
            "data": { "error": "Bad Request" },
            "success": false,
            "status": 400
        }));
    });

    let handles = test_state_with_imgur_api_base_url(imgur_server.base_url());
    let username = Username::from("octo");
    handles
        .store
        .add_user(User::new(
            username.clone(),
            42,
            "gh-token".to_string(),
            "gh-refresh".to_string(),
        ))
        .await?;
    let (actor, auth_token) = crate::domain::actors::Actor::new_for_user(username);
    handles.store.add_actor(actor).await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = client_with_token(auth_token);

    let patch = Patch::new(
        "asset failure".to_string(),
        "asset failure".to_string(),
        patch_diff(),
        PatchStatus::Open,
        false,
        None,
        Vec::new(),
        service_repo_name(),
        Some(GithubPr::new(
            "octo".to_string(),
            "repo".to_string(),
            42,
            None,
            None,
            None,
            None,
        )),
    );

    let created: UpsertPatchResponse = client
        .post(format!("{}/v1/patches", server.base_url()))
        .json(&UpsertPatchRequest::new(patch))
        .send()
        .await?
        .json()
        .await?;

    let response = client
        .post(format!(
            "{}/v1/patches/{}/assets?name=failure.png",
            server.base_url(),
            created.patch_id
        ))
        .header("content-type", "image/png")
        .body("binary-payload")
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let error: metis_common::api::v1::error::ApiErrorBody = response.json().await?;
    assert!(
        error
            .error
            .contains("imgur upload failed with status 400 Bad Request")
    );
    upload_mock.assert_hits(1);
    Ok(())
}

#[tokio::test]
async fn create_patch_asset_uploads_tiny_payload() -> anyhow::Result<()> {
    let imgur_server = MockServer::start_async().await;

    const TINY_PAYLOAD: &[u8] = b"tiny-payload";
    let upload_mock = imgur_server.mock(|when, then| {
        when.method(POST)
            .path("/3/image")
            .header("authorization", "Client-ID imgur-client-id")
            .matches(move |req: &HttpMockRequest| {
                let body = match req.body.as_ref() {
                    Some(body) => body,
                    None => return false,
                };
                body.windows(TINY_PAYLOAD.len())
                    .any(|window| window == TINY_PAYLOAD)
            });
        then.status(200).json_body(json!({
            "data": { "link": "https://i.imgur.com/asset-2.png" },
            "success": true,
            "status": 200
        }));
    });

    let handles = test_state_with_imgur_api_base_url(imgur_server.base_url());
    let username = Username::from("octo");
    handles
        .store
        .add_user(User::new(
            username.clone(),
            42,
            "gh-token".to_string(),
            "gh-refresh".to_string(),
        ))
        .await?;
    let (actor, auth_token) = crate::domain::actors::Actor::new_for_user(username);
    handles.store.add_actor(actor).await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = client_with_token(auth_token);

    let patch = Patch::new(
        "asset tiny".to_string(),
        "asset tiny".to_string(),
        patch_diff(),
        PatchStatus::Open,
        false,
        None,
        Vec::new(),
        service_repo_name(),
        Some(GithubPr::new(
            "octo".to_string(),
            "repo".to_string(),
            42,
            None,
            None,
            None,
            None,
        )),
    );

    let created: UpsertPatchResponse = client
        .post(format!("{}/v1/patches", server.base_url()))
        .json(&UpsertPatchRequest::new(patch))
        .send()
        .await?
        .json()
        .await?;

    let response: CreatePatchAssetResponse = client
        .post(format!(
            "{}/v1/patches/{}/assets?name=tiny.png",
            server.base_url(),
            created.patch_id
        ))
        .header("content-type", "image/png")
        .body(TINY_PAYLOAD)
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(response.asset_url, "https://i.imgur.com/asset-2.png");
    upload_mock.assert_hits(1);

    Ok(())
}

#[tokio::test]
async fn create_patch_asset_surfaces_imgur_bad_size() -> anyhow::Result<()> {
    let imgur_server = MockServer::start_async().await;

    let upload_mock = imgur_server.mock(|when, then| {
        when.method(POST)
            .path("/3/image")
            .header("authorization", "Client-ID imgur-client-id");
        then.status(422).json_body(json!({
            "data": { "error": "Bad Size" },
            "success": false,
            "status": 422
        }));
    });

    let handles = test_state_with_imgur_api_base_url(imgur_server.base_url());
    let username = Username::from("octo");
    handles
        .store
        .add_user(User::new(
            username.clone(),
            42,
            "gh-token".to_string(),
            "gh-refresh".to_string(),
        ))
        .await?;
    let (actor, auth_token) = crate::domain::actors::Actor::new_for_user(username);
    handles.store.add_actor(actor).await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = client_with_token(auth_token);

    let patch = Patch::new(
        "asset bad size".to_string(),
        "asset bad size".to_string(),
        patch_diff(),
        PatchStatus::Open,
        false,
        None,
        Vec::new(),
        service_repo_name(),
        Some(GithubPr::new(
            "octo".to_string(),
            "repo".to_string(),
            42,
            None,
            None,
            None,
            None,
        )),
    );

    let created: UpsertPatchResponse = client
        .post(format!("{}/v1/patches", server.base_url()))
        .json(&UpsertPatchRequest::new(patch))
        .send()
        .await?
        .json()
        .await?;

    let response = client
        .post(format!(
            "{}/v1/patches/{}/assets?name=bad-size.png",
            server.base_url(),
            created.patch_id
        ))
        .header("content-type", "image/png")
        .body("binary-payload")
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let error: metis_common::api::v1::error::ApiErrorBody = response.json().await?;
    assert!(
        error
            .error
            .contains("imgur upload failed with status 422 Unprocessable Entity")
    );
    upload_mock.assert_hits(1);
    Ok(())
}

#[tokio::test]
async fn create_patch_asset_uploads_without_github_pr() -> anyhow::Result<()> {
    let imgur_server = MockServer::start_async().await;
    let upload_mock = imgur_server.mock(|when, then| {
        when.method(POST)
            .path("/3/image")
            .header("authorization", "Client-ID imgur-client-id");
        then.status(200).json_body(json!({
            "data": { "link": "https://i.imgur.com/no-pr.png" },
            "success": true,
            "status": 200
        }));
    });

    let handles = test_state_with_imgur_api_base_url(imgur_server.base_url());
    let username = Username::from("octo");
    handles
        .store
        .add_user(User::new(
            username.clone(),
            42,
            "gh-token".to_string(),
            "gh-refresh".to_string(),
        ))
        .await?;
    let (actor, auth_token) = crate::domain::actors::Actor::new_for_user(username);
    handles.store.add_actor(actor).await?;

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = client_with_token(auth_token);
    let patch = Patch::new(
        "missing pr".to_string(),
        "missing pr".to_string(),
        patch_diff(),
        PatchStatus::Open,
        false,
        None,
        Vec::new(),
        service_repo_name(),
        None,
    );

    let created: UpsertPatchResponse = client
        .post(format!("{}/v1/patches", server.base_url()))
        .json(&UpsertPatchRequest::new(patch))
        .send()
        .await?
        .json()
        .await?;

    let response: CreatePatchAssetResponse = client
        .post(format!(
            "{}/v1/patches/{}/assets",
            server.base_url(),
            created.patch_id
        ))
        .body(vec![1, 2, 3])
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(response.asset_url, "https://i.imgur.com/no-pr.png");
    upload_mock.assert_hits(1);
    Ok(())
}

fn client_with_token(auth_token: String) -> Client {
    let mut headers = reqwest::header::HeaderMap::new();
    let auth_value = format!("Bearer {auth_token}");
    headers.insert(
        reqwest::header::AUTHORIZATION,
        reqwest::header::HeaderValue::from_str(&auth_value).expect("valid auth header"),
    );
    Client::builder()
        .default_headers(headers)
        .build()
        .expect("failed to build client")
}
