use crate::test_utils::{spawn_test_server, test_client};
use hydra_common::{
    IssueId, Versioned, WorkflowId,
    api::v1::{
        documents::{Document, UpsertDocumentRequest, UpsertDocumentResponse},
        workflows::{StartWorkflowRequest, TransitionWorkflowRequest, Workflow, WorkflowStatus},
    },
};
use reqwest::StatusCode;
use std::collections::HashMap;

const TEMPLATE_PATH: &str = "/workflows/route-test.yaml";

fn full_lifecycle_yaml() -> &'static str {
    r#"
name: "Route Test"
description: "Route-level test workflow."
initial_state: develop
context:
  - name: repo_name
    description: "Repository to work in"
    required: true
  - name: branch
    description: "Branch for the work"
    required: true

states:
  - id: develop
    name: "Development"
    on_enter:
      type: create_issue
      issue_type: task
      title_template: "Develop: {{workflow.name}}"
      description_template: "Implement on {{context.branch}}."
      assignee: "swe"
      session_settings:
        repo_name: "{{context.repo_name}}"
        branch: "{{context.branch}}"

  - id: review
    name: "Review"
    on_enter:
      type: create_issue
      issue_type: review-request
      title_template: "Review: {{workflow.name}}"
      description_template: "Review the diff."
      assignee: "reviewer"

  - id: merged
    name: "Merged"
    terminal: true
    terminal_status: closed
    on_enter:
      type: noop

transitions:
  - from: develop
    to: review
    label: "Ready for Review"
    trigger:
      type: explicit
      transition_id: ready-for-review
  - from: review
    to: merged
    label: "Merged"
    trigger:
      type: on_child_status
      status: closed
"#
}

fn sample_context() -> HashMap<String, String> {
    let mut ctx = HashMap::new();
    ctx.insert("repo_name".to_string(), "dourolabs/hydra".to_string());
    ctx.insert("branch".to_string(), "feature/widget".to_string());
    ctx
}

async fn upload_template(
    client: &reqwest::Client,
    base_url: &str,
    path: &str,
    yaml: &str,
) -> anyhow::Result<()> {
    let document = Document::new(
        "Route Test Template".to_string(),
        yaml.to_string(),
        Some(path.to_string()),
        None,
        false,
    )?;
    let _: UpsertDocumentResponse = client
        .post(format!("{base_url}/v1/documents"))
        .json(&UpsertDocumentRequest::new(document))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(())
}

#[tokio::test]
async fn create_workflow_returns_workflow_with_initial_state() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    upload_template(
        &client,
        &server.base_url(),
        TEMPLATE_PATH,
        full_lifecycle_yaml(),
    )
    .await?;

    let response = client
        .post(format!("{}/v1/workflows", server.base_url()))
        .json(&StartWorkflowRequest::new(
            TEMPLATE_PATH.to_string(),
            None,
            sample_context(),
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let workflow: Versioned<Workflow> = response.json().await?;
    assert_eq!(workflow.item.template_path, TEMPLATE_PATH);
    assert_eq!(workflow.item.current_state, "develop");
    assert_eq!(workflow.item.status, WorkflowStatus::Active);
    assert!(workflow.item.active_issue_id.is_some());
    assert_eq!(workflow.item.history.len(), 1);

    Ok(())
}

#[tokio::test]
async fn create_workflow_missing_required_context_returns_400() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    upload_template(
        &client,
        &server.base_url(),
        TEMPLATE_PATH,
        full_lifecycle_yaml(),
    )
    .await?;

    // `repo_name` is required; supply only `branch`.
    let mut ctx = HashMap::new();
    ctx.insert("branch".to_string(), "feature/widget".to_string());

    let response = client
        .post(format!("{}/v1/workflows", server.base_url()))
        .json(&StartWorkflowRequest::new(
            TEMPLATE_PATH.to_string(),
            None,
            ctx,
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn create_workflow_missing_template_returns_404() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let response = client
        .post(format!("{}/v1/workflows", server.base_url()))
        .json(&StartWorkflowRequest::new(
            "/workflows/does-not-exist.yaml".to_string(),
            None,
            HashMap::new(),
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn get_workflow_returns_created_workflow() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    upload_template(
        &client,
        &server.base_url(),
        TEMPLATE_PATH,
        full_lifecycle_yaml(),
    )
    .await?;

    let created: Versioned<Workflow> = client
        .post(format!("{}/v1/workflows", server.base_url()))
        .json(&StartWorkflowRequest::new(
            TEMPLATE_PATH.to_string(),
            None,
            sample_context(),
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let response = client
        .get(format!(
            "{}/v1/workflows/{}",
            server.base_url(),
            created.item.workflow_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let fetched: Versioned<Workflow> = response.json().await?;
    assert_eq!(fetched.item.workflow_id, created.item.workflow_id);
    assert_eq!(fetched.item.current_state, "develop");
    Ok(())
}

#[tokio::test]
async fn get_workflow_not_found_returns_404() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let fake_id = WorkflowId::new();
    let response = client
        .get(format!("{}/v1/workflows/{}", server.base_url(), fake_id))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn list_workflows_filters_by_status() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    upload_template(
        &client,
        &server.base_url(),
        TEMPLATE_PATH,
        full_lifecycle_yaml(),
    )
    .await?;

    let active: Versioned<Workflow> = client
        .post(format!("{}/v1/workflows", server.base_url()))
        .json(&StartWorkflowRequest::new(
            TEMPLATE_PATH.to_string(),
            None,
            sample_context(),
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let cancelled: Versioned<Workflow> = client
        .post(format!("{}/v1/workflows", server.base_url()))
        .json(&StartWorkflowRequest::new(
            TEMPLATE_PATH.to_string(),
            None,
            sample_context(),
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    client
        .delete(format!(
            "{}/v1/workflows/{}",
            server.base_url(),
            cancelled.item.workflow_id
        ))
        .send()
        .await?
        .error_for_status()?;

    // Filter: status=active should return only the active one.
    let response = client
        .get(format!("{}/v1/workflows?status=active", server.base_url()))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let listed: Vec<Versioned<Workflow>> = response.json().await?;
    let ids: Vec<&WorkflowId> = listed.iter().map(|w| &w.item.workflow_id).collect();
    assert!(
        ids.contains(&&active.item.workflow_id),
        "active workflow should appear in status=active listing"
    );
    assert!(
        !ids.contains(&&cancelled.item.workflow_id),
        "cancelled workflow should be filtered out of status=active listing"
    );

    // Filter: status=cancelled should return only the cancelled one.
    let response = client
        .get(format!(
            "{}/v1/workflows?status=cancelled",
            server.base_url()
        ))
        .send()
        .await?;
    let listed: Vec<Versioned<Workflow>> = response.json().await?;
    let ids: Vec<&WorkflowId> = listed.iter().map(|w| &w.item.workflow_id).collect();
    assert!(ids.contains(&&cancelled.item.workflow_id));
    assert!(!ids.contains(&&active.item.workflow_id));
    Ok(())
}

#[tokio::test]
async fn list_workflows_filters_by_issue_id() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    upload_template(
        &client,
        &server.base_url(),
        TEMPLATE_PATH,
        full_lifecycle_yaml(),
    )
    .await?;

    let workflow: Versioned<Workflow> = client
        .post(format!("{}/v1/workflows", server.base_url()))
        .json(&StartWorkflowRequest::new(
            TEMPLATE_PATH.to_string(),
            None,
            sample_context(),
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let child_issue_id = workflow
        .item
        .active_issue_id
        .clone()
        .expect("develop state creates a child issue");

    let response = client
        .get(format!(
            "{}/v1/workflows?issue_id={}",
            server.base_url(),
            child_issue_id
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let listed: Vec<Versioned<Workflow>> = response.json().await?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].item.workflow_id, workflow.item.workflow_id);

    // Filtering by an unassociated issue returns an empty list.
    let other_issue = IssueId::new();
    let response = client
        .get(format!(
            "{}/v1/workflows?issue_id={}",
            server.base_url(),
            other_issue
        ))
        .send()
        .await?;
    let listed: Vec<Versioned<Workflow>> = response.json().await?;
    assert!(listed.is_empty());
    Ok(())
}

#[tokio::test]
async fn transition_workflow_explicit_advances_state() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    upload_template(
        &client,
        &server.base_url(),
        TEMPLATE_PATH,
        full_lifecycle_yaml(),
    )
    .await?;

    let workflow: Versioned<Workflow> = client
        .post(format!("{}/v1/workflows", server.base_url()))
        .json(&StartWorkflowRequest::new(
            TEMPLATE_PATH.to_string(),
            None,
            sample_context(),
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let response = client
        .post(format!(
            "{}/v1/workflows/{}/transition",
            server.base_url(),
            workflow.item.workflow_id
        ))
        .json(&TransitionWorkflowRequest::new(
            "ready-for-review".to_string(),
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let after: Versioned<Workflow> = response.json().await?;
    assert_eq!(after.item.current_state, "review");
    assert_eq!(after.item.history.len(), 2);
    Ok(())
}

#[tokio::test]
async fn transition_workflow_non_explicit_trigger_returns_400() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    upload_template(
        &client,
        &server.base_url(),
        TEMPLATE_PATH,
        full_lifecycle_yaml(),
    )
    .await?;

    let workflow: Versioned<Workflow> = client
        .post(format!("{}/v1/workflows", server.base_url()))
        .json(&StartWorkflowRequest::new(
            TEMPLATE_PATH.to_string(),
            None,
            sample_context(),
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    // First move to `review` via the explicit trigger.
    client
        .post(format!(
            "{}/v1/workflows/{}/transition",
            server.base_url(),
            workflow.item.workflow_id
        ))
        .json(&TransitionWorkflowRequest::new(
            "ready-for-review".to_string(),
        ))
        .send()
        .await?
        .error_for_status()?;

    // From `review`, the only outgoing transition is `on_child_status`, so
    // the state has no Explicit transitions. The API must reject with 400.
    let response = client
        .post(format!(
            "{}/v1/workflows/{}/transition",
            server.base_url(),
            workflow.item.workflow_id
        ))
        .json(&TransitionWorkflowRequest::new("anything".to_string()))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn cancel_workflow_marks_it_cancelled() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    upload_template(
        &client,
        &server.base_url(),
        TEMPLATE_PATH,
        full_lifecycle_yaml(),
    )
    .await?;

    let workflow: Versioned<Workflow> = client
        .post(format!("{}/v1/workflows", server.base_url()))
        .json(&StartWorkflowRequest::new(
            TEMPLATE_PATH.to_string(),
            None,
            sample_context(),
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let response = client
        .delete(format!(
            "{}/v1/workflows/{}",
            server.base_url(),
            workflow.item.workflow_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let after: Versioned<Workflow> = response.json().await?;
    assert_eq!(after.item.status, WorkflowStatus::Cancelled);

    // A second cancel should fail with 409 (already terminal).
    let response = client
        .delete(format!(
            "{}/v1/workflows/{}",
            server.base_url(),
            workflow.item.workflow_id
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    Ok(())
}
