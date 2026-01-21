use super::common::{default_image, patch_diff, service_repo_name};
use crate::{
    domain::{
        issues::{
            Issue, IssueRecord, IssueStatus, IssueType, UpsertIssueRequest, UpsertIssueResponse,
        },
        jobs::BundleSpec,
        patches::{
            ListPatchesResponse, Patch, PatchRecord, PatchStatus, SearchPatchesQuery,
            UpsertPatchRequest, UpsertPatchResponse,
        },
    },
    store::Task,
    test_utils::{spawn_test_server, spawn_test_server_with_state, test_client, test_state},
};
use chrono::Utc;
use std::collections::HashMap;

#[tokio::test]
async fn patches_can_be_created_and_retrieved() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let patch = Patch {
        title: "Initial patch".to_string(),
        description: "initial patch".to_string(),
        diff: patch_diff(),
        status: PatchStatus::Open,
        is_automatic_backup: false,
        created_by: None,
        reviews: Vec::new(),
        service_repo_name: service_repo_name(),
        github: None,
    };

    let response = client
        .post(format!("{}/v1/patches", server.base_url()))
        .json(&UpsertPatchRequest {
            patch: patch.clone(),
        })
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
async fn creating_patch_with_created_by_links_job() -> anyhow::Result<()> {
    let state = test_state();
    let default_image = default_image();
    let job_id = super::common::task_id("t-emit");
    let store = state.store.clone();
    {
        let mut store_write = store.write().await;
        store_write
            .add_task_with_id(
                job_id.clone(),
                Task {
                    prompt: "0".to_string(),
                    context: BundleSpec::None,
                    spawned_from: None,
                    image: Some(default_image),
                    env_vars: HashMap::new(),
                },
                Utc::now(),
            )
            .await?;
        store_write.mark_task_running(&job_id, Utc::now()).await?;
    }

    let server = spawn_test_server_with_state(state).await?;
    let client = test_client();
    let response = client
        .post(format!("{}/v1/patches", server.base_url()))
        .json(&UpsertPatchRequest {
            patch: Patch {
                title: "artifact with creator".to_string(),
                description: "artifact with creator".to_string(),
                diff: patch_diff(),
                status: PatchStatus::Open,
                is_automatic_backup: false,
                created_by: Some(job_id.clone()),
                reviews: Vec::new(),
                service_repo_name: service_repo_name(),
                github: None,
            },
        })
        .send()
        .await?;

    assert!(response.status().is_success());
    let created: UpsertPatchResponse = response.json().await?;

    let patch = {
        let store_read = store.read().await;
        store_read.get_patch(&created.patch_id).await?
    };
    assert_eq!(patch.created_by, Some(job_id));
    Ok(())
}

#[tokio::test]
async fn closing_patch_closes_merge_request_issues() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let base_patch = Patch {
        title: "link patch to issue".to_string(),
        description: "issue-linked patch".to_string(),
        diff: patch_diff(),
        status: PatchStatus::Open,
        is_automatic_backup: false,
        created_by: None,
        reviews: Vec::new(),
        service_repo_name: service_repo_name(),
        github: None,
    };

    let created_patch: UpsertPatchResponse = client
        .post(format!("{}/v1/patches", server.base_url()))
        .json(&UpsertPatchRequest {
            patch: base_patch.clone(),
        })
        .send()
        .await?
        .json()
        .await?;

    let merge_request_issue = Issue {
        issue_type: IssueType::MergeRequest,
        description: "linked merge request".to_string(),
        creator: String::new(),
        progress: String::new(),
        status: IssueStatus::Open,
        assignee: None,
        todo_list: Vec::new(),
        dependencies: vec![],
        patches: vec![created_patch.patch_id.clone()],
    };

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
        .json(&UpsertPatchRequest {
            patch: merged_patch,
        })
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

    let patch = Patch {
        title: "refactor logging".to_string(),
        description: "refactor logging".to_string(),
        diff: patch_diff(),
        status: PatchStatus::Open,
        is_automatic_backup: false,
        created_by: None,
        reviews: Vec::new(),
        service_repo_name: service_repo_name(),
        github: None,
    };
    let filtered_patch = Patch {
        title: "login retry patch".to_string(),
        description: "login retry patch".to_string(),
        diff: patch_diff(),
        status: PatchStatus::Open,
        is_automatic_backup: false,
        created_by: None,
        reviews: Vec::new(),
        service_repo_name: service_repo_name(),
        github: None,
    };

    for patch in [patch.clone(), filtered_patch.clone()] {
        let response = client
            .post(format!("{}/v1/patches", server.base_url()))
            .json(&UpsertPatchRequest { patch })
            .send()
            .await?;
        assert!(response.status().is_success());
    }

    let patch_results: ListPatchesResponse = client
        .get(format!("{}/v1/patches", server.base_url()))
        .query(&SearchPatchesQuery {
            q: Some("login".to_string()),
        })
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(patch_results.patches.len(), 1);
    assert_eq!(patch_results.patches[0].patch, filtered_patch);
    Ok(())
}
