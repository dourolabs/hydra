use super::common::{default_image, patch_diff, service_repo_name};
use crate::{
    domain::{
        issues::{Issue, IssueStatus, IssueType},
        jobs::BundleSpec,
        patches::{GithubPr, Patch, PatchStatus},
        users::{User, Username},
    },
    store::{Status, Task},
    test_utils::{
        github_user_response, spawn_test_server, spawn_test_server_with_state, test_client,
        test_state_handles, test_state_with_github_api_base_url,
    },
};
use chrono::Utc;
use httpmock::prelude::HttpMockRequest;
use httpmock::{Method::GET, Method::POST, MockServer};
use metis_common::{
    PatchId,
    api::v1::{
        issues::{IssueVersionRecord, UpsertIssueRequest, UpsertIssueResponse},
        patches::{
            CreatePatchAssetResponse, ListPatchVersionsResponse, ListPatchesResponse,
            PatchVersionRecord, SearchPatchesQuery, UpsertPatchRequest, UpsertPatchResponse,
        },
    },
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
        .json(&UpsertPatchRequest::new(patch.clone().into()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let created: UpsertPatchResponse = response.json().await?;
    assert!(!created.patch_id.as_ref().is_empty());

    let fetched: PatchVersionRecord = client
        .get(format!(
            "{}/v1/patches/{}",
            server.base_url(),
            created.patch_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(fetched.patch_id, created.patch_id);
    let expected_patch: metis_common::api::v1::patches::Patch = patch.into();
    assert_eq!(fetched.patch, expected_patch);
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
        .json(&UpsertPatchRequest::new(patch.clone().into()))
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
        .json(&UpsertPatchRequest::new(updated_patch.into()))
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
        .json(&UpsertPatchRequest::new(patch.into()))
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
    let check_state = handles.state.clone();
    let (job_id, _) = handles
        .store
        .add_task(
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                creator: None,
                image: Some(default_image),
                model: None,
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
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
        .json(&UpsertPatchRequest::new(
            Patch::new(
                "artifact with creator".to_string(),
                "artifact with creator".to_string(),
                patch_diff(),
                PatchStatus::Open,
                false,
                Some(job_id.clone()),
                Vec::new(),
                service_repo_name(),
                None,
            )
            .into(),
        ))
        .send()
        .await?;

    assert!(response.status().is_success());
    let created: UpsertPatchResponse = response.json().await?;

    let patch = check_state.get_patch(&created.patch_id, false).await?;
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
        .json(&UpsertPatchRequest::new(base_patch.clone().into()))
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
        .json(&UpsertIssueRequest::new(merge_request_issue.into(), None))
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
        .json(&UpsertPatchRequest::new(merged_patch.into()))
        .send()
        .await?
        .error_for_status()?;

    let fetched_issue: IssueVersionRecord = client
        .get(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created_issue.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(
        fetched_issue.issue.status,
        metis_common::api::v1::issues::IssueStatus::Closed
    );
    Ok(())
}

#[tokio::test]
async fn closed_patch_fails_merge_request_issues() -> anyhow::Result<()> {
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
        .json(&UpsertPatchRequest::new(base_patch.clone().into()))
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
        .json(&UpsertIssueRequest::new(merge_request_issue.into(), None))
        .send()
        .await?
        .json()
        .await?;

    let mut closed_patch = base_patch.clone();
    closed_patch.status = PatchStatus::Closed;
    client
        .put(format!(
            "{}/v1/patches/{}",
            server.base_url(),
            created_patch.patch_id
        ))
        .json(&UpsertPatchRequest::new(closed_patch.into()))
        .send()
        .await?
        .error_for_status()?;

    let fetched_issue: IssueVersionRecord = client
        .get(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created_issue.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(
        fetched_issue.issue.status,
        metis_common::api::v1::issues::IssueStatus::Failed
    );
    Ok(())
}

#[tokio::test]
async fn changes_requested_closes_merge_request_issues() -> anyhow::Result<()> {
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
        .json(&UpsertPatchRequest::new(base_patch.clone().into()))
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
        .json(&UpsertIssueRequest::new(merge_request_issue.into(), None))
        .send()
        .await?
        .json()
        .await?;

    let mut changes_requested_patch = base_patch.clone();
    changes_requested_patch.status = PatchStatus::ChangesRequested;
    client
        .put(format!(
            "{}/v1/patches/{}",
            server.base_url(),
            created_patch.patch_id
        ))
        .json(&UpsertPatchRequest::new(changes_requested_patch.into()))
        .send()
        .await?
        .error_for_status()?;

    let fetched_issue: IssueVersionRecord = client
        .get(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created_issue.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(
        fetched_issue.issue.status,
        metis_common::api::v1::issues::IssueStatus::Failed
    );
    Ok(())
}

#[tokio::test]
async fn updating_changes_requested_patch_creates_merge_request_issue() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let server = spawn_test_server_with_state(handles.state.clone(), handles.store.clone()).await?;
    let client = test_client();

    let base_patch = Patch::new(
        "Needs changes".to_string(),
        "patch description".to_string(),
        patch_diff(),
        PatchStatus::ChangesRequested,
        false,
        None,
        Vec::new(),
        service_repo_name(),
        None,
    );

    let created_patch: UpsertPatchResponse = client
        .post(format!("{}/v1/patches", server.base_url()))
        .json(&UpsertPatchRequest::new(base_patch.clone().into()))
        .send()
        .await?
        .json()
        .await?;

    let merge_request_issue = Issue::new(
        IssueType::MergeRequest,
        "previous merge request".to_string(),
        Username::from("creator"),
        String::new(),
        IssueStatus::Closed,
        Some("agent-a".to_string()),
        None,
        Vec::new(),
        vec![],
        vec![created_patch.patch_id.clone()],
    );

    client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(merge_request_issue.into(), None))
        .send()
        .await?
        .error_for_status()?;

    let mut updated_patch = base_patch.clone();
    updated_patch.title = "Updated patch".to_string();
    updated_patch.status = PatchStatus::Open;

    let updated_response: UpsertPatchResponse = client
        .put(format!(
            "{}/v1/patches/{}",
            server.base_url(),
            created_patch.patch_id
        ))
        .json(&UpsertPatchRequest::new(updated_patch.into()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert_eq!(updated_response.patch_id, created_patch.patch_id);

    let issue_ids = handles
        .store
        .get_issues_for_patch(&created_patch.patch_id)
        .await?;
    let mut merge_request_issues = Vec::new();
    for issue_id in issue_ids {
        let issue = handles.store.get_issue(&issue_id, false).await?;
        if issue.item.issue_type == IssueType::MergeRequest {
            merge_request_issues.push(issue);
        }
    }

    assert_eq!(merge_request_issues.len(), 2);
    let open_issue = merge_request_issues
        .iter()
        .find(|issue| issue.item.status == IssueStatus::Open)
        .expect("expected an open merge-request issue");
    assert!(open_issue.item.description.contains("Updated patch"));
    assert_eq!(open_issue.item.assignee.as_deref(), Some("agent-a"));

    Ok(())
}

#[tokio::test]
async fn reopening_changes_requested_patch_reuses_patch_and_opens_new_issue() -> anyhow::Result<()>
{
    let handles = test_state_handles();
    let server = spawn_test_server_with_state(handles.state.clone(), handles.store.clone()).await?;
    let client = test_client();

    let base_patch = Patch::new(
        "Review patch".to_string(),
        "patch description".to_string(),
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
        .json(&UpsertPatchRequest::new(base_patch.clone().into()))
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
        Some("agent-a".to_string()),
        None,
        Vec::new(),
        vec![],
        vec![created_patch.patch_id.clone()],
    );

    let created_issue: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(merge_request_issue.into(), None))
        .send()
        .await?
        .json()
        .await?;

    let mut changes_requested_patch = base_patch.clone();
    changes_requested_patch.status = PatchStatus::ChangesRequested;
    let changes_response: UpsertPatchResponse = client
        .put(format!(
            "{}/v1/patches/{}",
            server.base_url(),
            created_patch.patch_id
        ))
        .json(&UpsertPatchRequest::new(changes_requested_patch.into()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert_eq!(changes_response.patch_id, created_patch.patch_id);

    let fetched_issue: IssueVersionRecord = client
        .get(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created_issue.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(
        fetched_issue.issue.status,
        metis_common::api::v1::issues::IssueStatus::Failed
    );

    let mut reopened_patch = base_patch.clone();
    reopened_patch.title = "Review patch v2".to_string();
    reopened_patch.status = PatchStatus::Open;
    let reopened_response: UpsertPatchResponse = client
        .put(format!(
            "{}/v1/patches/{}",
            server.base_url(),
            created_patch.patch_id
        ))
        .json(&UpsertPatchRequest::new(reopened_patch.into()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert_eq!(reopened_response.patch_id, created_patch.patch_id);

    let issue_ids = handles
        .store
        .get_issues_for_patch(&created_patch.patch_id)
        .await?;
    let mut merge_request_issues = Vec::new();
    for issue_id in issue_ids {
        let issue = handles.store.get_issue(&issue_id, false).await?;
        if issue.item.issue_type == IssueType::MergeRequest {
            merge_request_issues.push(issue);
        }
    }

    // 3 MergeRequest issues: one auto-created on PatchCreated, one manually created
    // ("linked merge request"), and one auto-created on reopen (ChangesRequested -> Open).
    assert_eq!(merge_request_issues.len(), 3);
    let open_issue = merge_request_issues
        .iter()
        .find(|issue| issue.item.status == IssueStatus::Open)
        .expect("expected an open merge-request issue");
    assert!(open_issue.item.description.contains("Review patch v2"));
    assert_eq!(open_issue.item.assignee.as_deref(), Some("agent-a"));

    Ok(())
}

#[tokio::test]
async fn updating_open_patch_does_not_create_merge_request_issue() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let server = spawn_test_server_with_state(handles.state.clone(), handles.store.clone()).await?;
    let client = test_client();

    let base_patch = Patch::new(
        "Open patch".to_string(),
        "patch description".to_string(),
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
        .json(&UpsertPatchRequest::new(base_patch.clone().into()))
        .send()
        .await?
        .json()
        .await?;

    let merge_request_issue = Issue::new(
        IssueType::MergeRequest,
        "previous merge request".to_string(),
        Username::from("creator"),
        String::new(),
        IssueStatus::Closed,
        Some("agent-a".to_string()),
        None,
        Vec::new(),
        vec![],
        vec![created_patch.patch_id.clone()],
    );

    client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(merge_request_issue.into(), None))
        .send()
        .await?
        .error_for_status()?;

    let mut updated_patch = base_patch.clone();
    updated_patch.title = "Updated patch".to_string();

    client
        .put(format!(
            "{}/v1/patches/{}",
            server.base_url(),
            created_patch.patch_id
        ))
        .json(&UpsertPatchRequest::new(updated_patch.into()))
        .send()
        .await?
        .error_for_status()?;

    let issue_ids = handles
        .store
        .get_issues_for_patch(&created_patch.patch_id)
        .await?;
    let mut merge_request_issues = Vec::new();
    for issue_id in issue_ids {
        let issue = handles.store.get_issue(&issue_id, false).await?;
        if issue.item.issue_type == IssueType::MergeRequest {
            merge_request_issues.push(issue);
        }
    }

    // There should be 2 MergeRequest issues: the one auto-created by the PatchCreated
    // automation, and the manually-created "previous merge request". The patch update
    // (Open -> Open title change) should NOT have created a third one.
    assert_eq!(merge_request_issues.len(), 2);
    let existing_issue = merge_request_issues
        .iter()
        .find(|issue| issue.item.status == IssueStatus::Closed)
        .expect("expected a closed merge-request issue");
    assert!(
        existing_issue
            .item
            .description
            .contains("previous merge request")
    );
    assert_eq!(existing_issue.item.assignee.as_deref(), Some("agent-a"));

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
            .json(&UpsertPatchRequest::new(patch.into()))
            .send()
            .await?;
        assert!(response.status().is_success());
    }

    let patch_results: ListPatchesResponse = client
        .get(format!("{}/v1/patches", server.base_url()))
        .query(&SearchPatchesQuery::new(Some("login".to_string()), None))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(patch_results.patches.len(), 1);
    let expected_patch: metis_common::api::v1::patches::Patch = filtered_patch.into();
    assert_eq!(patch_results.patches[0].patch, expected_patch);
    Ok(())
}

#[tokio::test]
async fn create_patch_asset_uploads_to_github() -> anyhow::Result<()> {
    let github_server = MockServer::start_async().await;
    let _user_mock = github_server.mock(|when, then| {
        when.method(GET).path("/user");
        then.status(200).json_body(github_user_response("octo", 42));
    });

    let upload_mock = github_server.mock(|when, then| {
        when.method(POST)
            .path("/repos/octo/repo/issues/42/comments/attachments")
            .query_param("name", "screenshot.png")
            .header("authorization", "Bearer gh-token")
            .header("content-type", "image/png")
            .body("binary-payload");
        then.status(201)
            .json_body(json!({ "url": "https://github.com/octo/repo/assets/1" }));
    });

    let handles = test_state_with_github_api_base_url(github_server.base_url());
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
        .json(&UpsertPatchRequest::new(patch.into()))
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

    assert_eq!(response.asset_url, "https://github.com/octo/repo/assets/1");
    upload_mock.assert_hits(1);
    Ok(())
}

#[tokio::test]
async fn create_patch_asset_surfaces_github_400() -> anyhow::Result<()> {
    let github_server = MockServer::start_async().await;
    let _user_mock = github_server.mock(|when, then| {
        when.method(GET).path("/user");
        then.status(200).json_body(github_user_response("octo", 42));
    });

    let upload_mock = github_server.mock(|when, then| {
        when.method(POST)
            .path("/repos/octo/repo/issues/42/comments/attachments")
            .query_param("name", "failure.png")
            .header("authorization", "Bearer gh-token")
            .header("content-type", "image/png")
            .body("binary-payload");
        then.status(400)
            .json_body(json!({ "message": "Bad Request" }));
    });

    let handles = test_state_with_github_api_base_url(github_server.base_url());
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
        .json(&UpsertPatchRequest::new(patch.into()))
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
            .contains("github asset upload failed with status 400 Bad Request")
    );
    upload_mock.assert_hits(1);
    Ok(())
}

#[tokio::test]
async fn create_patch_asset_sets_content_length_for_tiny_payload() -> anyhow::Result<()> {
    let github_server = MockServer::start_async().await;
    let _user_mock = github_server.mock(|when, then| {
        when.method(GET).path("/user");
        then.status(200).json_body(github_user_response("octo", 42));
    });

    const TINY_PAYLOAD: &[u8] = b"tiny-payload";
    let upload_mock = github_server.mock(|when, then| {
        when.method(POST)
            .path("/repos/octo/repo/issues/42/comments/attachments")
            .query_param("name", "tiny.png")
            .header("authorization", "Bearer gh-token")
            .matches(move |req: &HttpMockRequest| {
                let body = match req.body.as_ref() {
                    Some(body) => body,
                    None => return false,
                };
                let content_length = match req
                    .headers
                    .as_ref()
                    .and_then(|headers| {
                        headers
                            .iter()
                            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    })
                    .and_then(|(_, value)| value.parse::<usize>().ok())
                {
                    Some(value) => value,
                    None => return false,
                };
                if content_length != body.len() {
                    return false;
                }
                body.as_slice() == TINY_PAYLOAD
            });
        then.status(201)
            .json_body(json!({ "url": "https://github.com/octo/repo/assets/2" }));
    });

    let handles = test_state_with_github_api_base_url(github_server.base_url());
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
        .json(&UpsertPatchRequest::new(patch.into()))
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

    assert_eq!(response.asset_url, "https://github.com/octo/repo/assets/2");
    upload_mock.assert_hits(1);

    Ok(())
}

#[tokio::test]
async fn create_patch_asset_surfaces_github_bad_size() -> anyhow::Result<()> {
    let github_server = MockServer::start_async().await;
    let _user_mock = github_server.mock(|when, then| {
        when.method(GET).path("/user");
        then.status(200).json_body(github_user_response("octo", 42));
    });

    let upload_mock = github_server.mock(|when, then| {
        when.method(POST)
            .path("/repos/octo/repo/issues/42/comments/attachments")
            .query_param("name", "bad-size.png")
            .header("authorization", "Bearer gh-token");
        then.status(422).json_body(json!({ "message": "Bad Size" }));
    });

    let handles = test_state_with_github_api_base_url(github_server.base_url());
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
        .json(&UpsertPatchRequest::new(patch.into()))
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
            .contains("github asset upload failed with status 422 Unprocessable Entity")
    );
    upload_mock.assert_hits(1);
    Ok(())
}

#[tokio::test]
async fn create_patch_asset_errors_without_github_pr() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
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
        .json(&UpsertPatchRequest::new(patch.into()))
        .send()
        .await?
        .json()
        .await?;

    let response = client
        .post(format!(
            "{}/v1/patches/{}/assets",
            server.base_url(),
            created.patch_id
        ))
        .body(vec![1, 2, 3])
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
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

// ===== Deletion Tests =====

#[tokio::test]
async fn delete_patch_basic_operation() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create a patch
    let patch = Patch::new(
        "patch to delete".to_string(),
        "patch description".to_string(),
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
        .json(&UpsertPatchRequest::new(patch.into()))
        .send()
        .await?
        .json()
        .await?;

    // Delete the patch
    let deleted: PatchVersionRecord = client
        .delete(format!(
            "{}/v1/patches/{}",
            server.base_url(),
            created.patch_id
        ))
        .send()
        .await?
        .json()
        .await?;

    // Verify the response has deleted=true
    assert!(deleted.patch.deleted);

    // Verify listing excludes the deleted patch
    let list: ListPatchesResponse = client
        .get(format!("{}/v1/patches", server.base_url()))
        .send()
        .await?
        .json()
        .await?;

    assert!(!list.patches.iter().any(|p| p.patch_id == created.patch_id));

    Ok(())
}

#[tokio::test]
async fn delete_patch_include_deleted_in_listing() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create and delete a patch
    let patch = Patch::new(
        "deleted patch".to_string(),
        "patch description".to_string(),
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
        .json(&UpsertPatchRequest::new(patch.into()))
        .send()
        .await?
        .json()
        .await?;

    client
        .delete(format!(
            "{}/v1/patches/{}",
            server.base_url(),
            created.patch_id
        ))
        .send()
        .await?
        .error_for_status()?;

    // List without include_deleted - verify not present
    let list_without: ListPatchesResponse = client
        .get(format!("{}/v1/patches", server.base_url()))
        .send()
        .await?
        .json()
        .await?;

    assert!(
        !list_without
            .patches
            .iter()
            .any(|p| p.patch_id == created.patch_id)
    );

    // List with include_deleted=true - verify present with deleted=true
    let list_with: ListPatchesResponse = client
        .get(format!("{}/v1/patches", server.base_url()))
        .query(&SearchPatchesQuery::new(None, Some(true)))
        .send()
        .await?
        .json()
        .await?;

    let deleted_patch = list_with
        .patches
        .iter()
        .find(|p| p.patch_id == created.patch_id);

    assert!(deleted_patch.is_some());
    assert!(deleted_patch.unwrap().patch.deleted);

    Ok(())
}

#[tokio::test]
async fn delete_patch_get_deleted_by_id() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create and delete a patch
    let patch = Patch::new(
        "get deleted patch".to_string(),
        "patch description".to_string(),
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
        .json(&UpsertPatchRequest::new(patch.into()))
        .send()
        .await?
        .json()
        .await?;

    client
        .delete(format!(
            "{}/v1/patches/{}",
            server.base_url(),
            created.patch_id
        ))
        .send()
        .await?
        .error_for_status()?;

    // GET by ID should return 404 for deleted patches
    let response = client
        .get(format!(
            "{}/v1/patches/{}",
            server.base_url(),
            created.patch_id
        ))
        .send()
        .await?;

    assert_eq!(response.status().as_u16(), 404);

    Ok(())
}

#[tokio::test]
async fn delete_patch_version_history() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create patch (v1)
    let patch = Patch::new(
        "version history patch".to_string(),
        "patch description".to_string(),
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
        .json(&UpsertPatchRequest::new(patch.into()))
        .send()
        .await?
        .json()
        .await?;

    // Update patch (v2)
    let updated_patch = Patch::new(
        "updated patch".to_string(),
        "updated description".to_string(),
        patch_diff(),
        PatchStatus::Open,
        false,
        None,
        Vec::new(),
        service_repo_name(),
        None,
    );

    client
        .put(format!(
            "{}/v1/patches/{}",
            server.base_url(),
            created.patch_id
        ))
        .json(&UpsertPatchRequest::new(updated_patch.into()))
        .send()
        .await?
        .error_for_status()?;

    // Delete patch (v3)
    client
        .delete(format!(
            "{}/v1/patches/{}",
            server.base_url(),
            created.patch_id
        ))
        .send()
        .await?
        .error_for_status()?;

    // Get versions - verify deletion creates new version with deleted=true
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

    assert_eq!(versions.versions.len(), 3);
    assert!(!versions.versions[0].patch.deleted);
    assert!(!versions.versions[1].patch.deleted);
    assert!(versions.versions[2].patch.deleted);

    Ok(())
}

#[tokio::test]
async fn delete_patch_idempotency() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create and delete a patch
    let patch = Patch::new(
        "idempotency patch".to_string(),
        "patch description".to_string(),
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
        .json(&UpsertPatchRequest::new(patch.into()))
        .send()
        .await?
        .json()
        .await?;

    // First delete
    let first_delete = client
        .delete(format!(
            "{}/v1/patches/{}",
            server.base_url(),
            created.patch_id
        ))
        .send()
        .await?;

    assert!(first_delete.status().is_success());

    // Second delete - should return 200 (idempotent)
    let second_delete = client
        .delete(format!(
            "{}/v1/patches/{}",
            server.base_url(),
            created.patch_id
        ))
        .send()
        .await?;

    assert!(second_delete.status().is_success());

    Ok(())
}

#[tokio::test]
async fn delete_patch_non_existent() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Attempt to delete non-existent ID
    let missing: PatchId = "p-nonexistent".parse().expect("valid patch id");
    let response = client
        .delete(format!("{}/v1/patches/{}", server.base_url(), missing))
        .send()
        .await?;

    // Verify 404 response
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}
