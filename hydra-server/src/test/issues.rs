use crate::{
    domain::{
        issues::{Issue, IssueDependency, IssueDependencyType, IssueType},
        users::Username,
    },
    test_utils::{spawn_test_server, test_client},
};
use hydra_common::{
    IssueId, PatchId,
    api::v1::{
        form::{Action, ActionStyle, Effect, Field, Form, Input, SelectOption},
        issues::{
            FormValidationError, IssueVersionRecord, ListIssueVersionsResponse, ListIssuesResponse,
            SearchIssuesQuery, SubmitFeedbackRequest, SubmitFormRequest, SubmitFormResponse,
            UpsertIssueRequest, UpsertIssueResponse,
        },
        projects::StatusKey,
    },
    test_utils::status::status,
};
use reqwest::StatusCode;
use serde_json::json;
use std::collections::HashMap;

fn issue(
    issue_type: IssueType,
    description: &str,
    creator: Username,
    progress: String,
    status: StatusKey,
    assignee: Option<&str>,
    dependencies: Vec<IssueDependency>,
    patches: Vec<PatchId>,
) -> Issue {
    // Server-side `principal_exists` validation rejects unknown User/Agent
    // assignees, so this fixture defaults to a `Principal::External`
    // (format-checked only, no DB lookup).
    use hydra_common::principal::{ExternalSystem, Principal};
    let assignee_principal = assignee.map(|name| Principal::External {
        system: ExternalSystem::try_new("test").expect("static external system"),
        username: name.to_string(),
    });
    Issue::new(
        issue_type,
        "Test Title".to_string(),
        description.to_string(),
        creator,
        progress,
        status,
        crate::domain::projects::default_project_id(),
        assignee_principal,
        None,
        dependencies,
        patches,
        None,
        None,
        None,
    )
}

fn user(username: &str) -> Username {
    Username::from(username)
}

fn default_user() -> Username {
    user("creator")
}

fn missing_user() -> Username {
    Username::from("")
}

#[tokio::test]
async fn update_issue_replaces_existing_value() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "original details".to_string(),
                default_user(),
                "Initial progress".to_string(),
                status("open"),
                crate::domain::projects::default_project_id(),
                None,
                None,
                vec![],
                Vec::new(),
                None,
                None,
                None,
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let updated: UpsertIssueResponse = client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .json(&UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "updated details".to_string(),
                default_user(),
                "Updated progress".to_string(),
                status("in-progress"),
                crate::domain::projects::default_project_id(),
                None,
                None,
                vec![],
                Vec::new(),
                None,
                None,
                None,
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(updated.issue_id, created.issue_id);
    Ok(())
}

#[tokio::test]
async fn issue_versions_endpoints_return_history() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "initial".to_string(),
                default_user(),
                "Initial progress".to_string(),
                status("open"),
                crate::domain::projects::default_project_id(),
                None,
                None,
                vec![],
                Vec::new(),
                None,
                None,
                None,
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let _updated: UpsertIssueResponse = client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .json(&UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "updated".to_string(),
                default_user(),
                "Updated progress".to_string(),
                status("in-progress"),
                crate::domain::projects::default_project_id(),
                None,
                None,
                vec![],
                Vec::new(),
                None,
                None,
                None,
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let versions: ListIssueVersionsResponse = client
        .get(format!(
            "{}/v1/issues/{}/versions",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(versions.versions.len(), 2);
    assert_eq!(versions.versions[0].issue_id, created.issue_id);
    assert_eq!(versions.versions[0].version, 1);
    assert_eq!(versions.versions[0].issue.description, "initial");
    assert_eq!(versions.versions[1].issue_id, created.issue_id);
    assert_eq!(versions.versions[1].version, 2);
    assert_eq!(versions.versions[1].issue.description, "updated");

    let version: IssueVersionRecord = client
        .get(format!(
            "{}/v1/issues/{}/versions/2",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(version.version, 2);
    assert_eq!(version.issue_id, created.issue_id);
    assert_eq!(version.issue.description, "updated");

    Ok(())
}

#[tokio::test]
async fn issue_version_endpoints_return_404s() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let missing: IssueId = "i-missing".parse().expect("valid issue id");
    let response = client
        .get(format!(
            "{}/v1/issues/{}/versions",
            server.base_url(),
            missing
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "initial".to_string(),
                default_user(),
                "Initial progress".to_string(),
                status("open"),
                crate::domain::projects::default_project_id(),
                None,
                None,
                vec![],
                Vec::new(),
                None,
                None,
                None,
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let response = client
        .get(format!(
            "{}/v1/issues/{}/versions/99",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn create_issue_rejects_missing_creator_with_parent() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let parent_creator = user("parent-creator");
    let parent: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "parent",
                parent_creator.clone(),
                String::new(),
                status("open"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let child_dependencies = vec![IssueDependency::new(
        IssueDependencyType::ChildOf,
        parent.issue_id.clone(),
    )];

    // Creating a child with a missing creator should be rejected
    let response = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "child",
                missing_user(),
                String::new(),
                status("open"),
                None,
                child_dependencies.clone(),
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn create_issue_rejects_missing_creator_without_parent() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let response = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "missing creator",
                missing_user(),
                String::new(),
                status("open"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn update_issue_rejects_closing_when_blocked() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let blocker: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "blocker",
                default_user(),
                String::new(),
                status("open"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let blocked_dependencies = vec![IssueDependency::new(
        IssueDependencyType::BlockedOn,
        blocker.issue_id.clone(),
    )];
    let blocked: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "blocked",
                default_user(),
                String::new(),
                status("open"),
                None,
                blocked_dependencies.clone(),
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let response = client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            blocked.issue_id
        ))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "blocked",
                default_user(),
                String::new(),
                status("closed"),
                None,
                blocked_dependencies.clone(),
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn update_issue_rejects_closing_with_open_children() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let parent: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "parent",
                default_user(),
                String::new(),
                status("open"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let child_dependencies = vec![IssueDependency::new(
        IssueDependencyType::ChildOf,
        parent.issue_id.clone(),
    )];
    let _child: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "child",
                default_user(),
                String::new(),
                status("open"),
                None,
                child_dependencies.clone(),
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let response = client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            parent.issue_id
        ))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "parent",
                default_user(),
                String::new(),
                status("closed"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn list_issues_supports_filters() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let base_issue = issue(
        IssueType::Bug,
        "login fails for guests",
        default_user(),
        String::new(),
        status("open"),
        None,
        vec![],
        Vec::new(),
    );
    let assigned_issue = issue(
        IssueType::Task,
        "assigned issue",
        default_user(),
        String::new(),
        status("open"),
        Some("owner-1"),
        vec![],
        Vec::new(),
    );
    let closed_issue = issue(
        IssueType::Task,
        "retire old endpoint",
        default_user(),
        String::new(),
        status("closed"),
        None,
        vec![],
        Vec::new(),
    );

    for issue in [
        base_issue.clone(),
        assigned_issue.clone(),
        closed_issue.clone(),
    ] {
        let response = client
            .post(format!("{}/v1/issues", server.base_url()))
            .json(&UpsertIssueRequest::new(issue.into(), None))
            .send()
            .await?;
        assert!(response.status().is_success());
    }

    let filtered_issues: ListIssuesResponse = client
        .get(format!("{}/v1/issues", server.base_url()))
        .query(&SearchIssuesQuery::new(
            Some(hydra_common::api::v1::issues::IssueType::Bug),
            vec![],
            None,
            None,
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    // List responses inline the server-resolved `StatusDefinition` on
    // `status`; mirror that on the expected summary so structural
    // equality holds.
    let expected_summary = |issue: Issue| {
        let input: hydra_common::api::v1::issues::IssueInput = issue.into();
        let resolved = crate::domain::projects::default_project_seed()
            .find_status(&input.status)
            .expect("default project covers all legacy status keys")
            .clone();
        let api_issue = hydra_common::api::v1::issues::Issue::new(
            input.issue_type,
            input.title,
            input.description,
            input.creator,
            input.progress,
            resolved,
            input.project_id,
            input.assignee,
            Some(input.session_settings),
            input.dependencies,
            input.patches,
            input.deleted,
            input.form,
            input.form_response,
            input.feedback,
        );
        hydra_common::api::v1::issues::IssueSummary::from(&api_issue)
    };

    assert_eq!(filtered_issues.issues.len(), 1);
    assert_eq!(
        filtered_issues.issues[0].issue,
        expected_summary(base_issue)
    );

    use hydra_common::principal::{ExternalSystem, Principal as ActorPrincipal};
    let owner_1 = ActorPrincipal::External {
        system: ExternalSystem::try_new("test").unwrap(),
        username: "owner-1".to_string(),
    };
    let filtered_by_assignee: ListIssuesResponse = client
        .get(format!("{}/v1/issues", server.base_url()))
        .query(&SearchIssuesQuery::new(
            None,
            vec![],
            Some(owner_1),
            None,
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(filtered_by_assignee.issues.len(), 1);
    assert_eq!(
        filtered_by_assignee.issues[0].issue,
        expected_summary(assigned_issue)
    );

    let filtered_by_status: ListIssuesResponse = client
        .get(format!("{}/v1/issues", server.base_url()))
        .query(&SearchIssuesQuery::new(
            None,
            vec![status("closed")],
            None,
            None,
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(filtered_by_status.issues.len(), 1);
    assert_eq!(
        filtered_by_status.issues[0].issue,
        expected_summary(closed_issue)
    );
    Ok(())
}

// ===== Deletion Tests =====

#[tokio::test]
async fn delete_issue_basic_operation() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create an issue
    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "issue to delete",
                default_user(),
                String::new(),
                status("open"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    // Delete the issue
    let deleted: IssueVersionRecord = client
        .delete(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;

    // Verify the response has deleted=true
    assert!(deleted.issue.deleted);

    // Verify listing excludes the deleted issue
    let list: ListIssuesResponse = client
        .get(format!("{}/v1/issues", server.base_url()))
        .send()
        .await?
        .json()
        .await?;

    assert!(!list.issues.iter().any(|i| i.issue_id == created.issue_id));

    Ok(())
}

#[tokio::test]
async fn delete_issue_include_deleted_in_listing() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create and delete an issue
    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "deleted issue",
                default_user(),
                String::new(),
                status("open"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    client
        .delete(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .error_for_status()?;

    // List without include_deleted - verify not present
    let list_without: ListIssuesResponse = client
        .get(format!("{}/v1/issues", server.base_url()))
        .send()
        .await?
        .json()
        .await?;

    assert!(
        !list_without
            .issues
            .iter()
            .any(|i| i.issue_id == created.issue_id)
    );

    // List with include_deleted=true - verify present with deleted=true
    let list_with: ListIssuesResponse = client
        .get(format!("{}/v1/issues", server.base_url()))
        .query(&SearchIssuesQuery::new(
            None,
            vec![],
            None,
            None,
            Some(true),
        ))
        .send()
        .await?
        .json()
        .await?;

    let deleted_issue = list_with
        .issues
        .iter()
        .find(|i| i.issue_id == created.issue_id);

    assert!(deleted_issue.is_some());
    assert!(deleted_issue.unwrap().issue.deleted);

    Ok(())
}

#[tokio::test]
async fn delete_issue_get_deleted_by_id() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create and delete an issue
    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "get deleted issue",
                default_user(),
                String::new(),
                status("open"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    client
        .delete(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .error_for_status()?;

    // GET by ID should return 404 for deleted issues
    let response = client
        .get(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?;

    assert_eq!(response.status().as_u16(), 404);

    Ok(())
}

#[tokio::test]
async fn delete_issue_idempotency() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create and delete an issue
    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "idempotency test",
                default_user(),
                String::new(),
                status("open"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    // First delete
    let first_delete = client
        .delete(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?;

    assert!(first_delete.status().is_success());

    // Second delete - should return 200 (idempotent)
    let second_delete = client
        .delete(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?;

    assert!(second_delete.status().is_success());

    Ok(())
}

#[tokio::test]
async fn delete_issue_non_existent() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Attempt to delete non-existent ID
    let missing: IssueId = "i-nonexistent".parse().expect("valid issue id");
    let response = client
        .delete(format!("{}/v1/issues/{}", server.base_url(), missing))
        .send()
        .await?;

    // Verify 404 response
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

// ===== Negative Version Offset Tests =====

#[tokio::test]
async fn get_issue_version_negative_offset_returns_correct_version() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create issue (v1)
    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "version one",
                default_user(),
                String::new(),
                status("open"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    // Update issue (v2)
    client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "version two",
                default_user(),
                String::new(),
                status("in-progress"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .error_for_status()?;

    // Update issue (v3)
    client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "version three",
                default_user(),
                String::new(),
                status("in-progress"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .error_for_status()?;

    // version=-1 should return v2 (second-to-last, i.e. max_version + (-1) = 3 + (-1) = 2)
    let v_minus_1: IssueVersionRecord = client
        .get(format!(
            "{}/v1/issues/{}/versions/-1",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(v_minus_1.version, 2);
    assert_eq!(v_minus_1.issue.description, "version two");

    // version=-2 should return v1
    let v_minus_2: IssueVersionRecord = client
        .get(format!(
            "{}/v1/issues/{}/versions/-2",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(v_minus_2.version, 1);
    assert_eq!(v_minus_2.issue.description, "version one");

    // Positive versions still work
    let v_positive: IssueVersionRecord = client
        .get(format!(
            "{}/v1/issues/{}/versions/3",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(v_positive.version, 3);
    assert_eq!(v_positive.issue.description, "version three");

    Ok(())
}

#[tokio::test]
async fn get_issue_version_zero_returns_400() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "test",
                default_user(),
                String::new(),
                status("open"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let response = client
        .get(format!(
            "{}/v1/issues/{}/versions/0",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn get_issue_version_out_of_range_negative_offset_returns_400() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create a single-version issue
    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "only version",
                default_user(),
                String::new(),
                status("open"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    // -1 on a single-version issue resolves to version 0 which is < 1
    let response = client
        .get(format!(
            "{}/v1/issues/{}/versions/-1",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: serde_json::Value = response.json().await?;
    let error = body["error"].as_str().unwrap();
    assert!(
        error.contains("out of range"),
        "expected out-of-range message, got: {error}"
    );

    // -100 is also out of range
    let response = client
        .get(format!(
            "{}/v1/issues/{}/versions/-100",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn list_issues_count_true_returns_total_count() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create 3 issues
    for desc in ["first", "second", "third"] {
        client
            .post(format!("{}/v1/issues", server.base_url()))
            .json(&UpsertIssueRequest::new(
                issue(
                    IssueType::Task,
                    desc,
                    default_user(),
                    String::new(),
                    status("open"),
                    None,
                    vec![],
                    Vec::new(),
                )
                .into(),
                None,
            ))
            .send()
            .await?;
    }

    // Without count param, total_count should be absent
    let resp: ListIssuesResponse = client
        .get(format!("{}/v1/issues?limit=2", server.base_url()))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(resp.issues.len(), 2);
    assert!(resp.total_count.is_none());

    // With count=true, total_count should be present and equal 3
    let resp: ListIssuesResponse = client
        .get(format!(
            "{}/v1/issues?limit=2&count=true",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(resp.issues.len(), 2);
    assert_eq!(resp.total_count, Some(3));

    Ok(())
}

fn test_form() -> Form {
    Form {
        prompt: "Please answer these questions".to_string(),
        fields: vec![
            Field {
                key: "name".to_string(),
                label: "Name".to_string(),
                description: None,
                input: Input::Text {
                    placeholder: None,
                    min_length: Some(1),
                    max_length: Some(50),
                    pattern: None,
                },
                default: None,
            },
            Field {
                key: "env".to_string(),
                label: "Environment".to_string(),
                description: None,
                input: Input::Select {
                    options: vec![
                        SelectOption {
                            value: "staging".to_string(),
                            label: "Staging".to_string(),
                        },
                        SelectOption {
                            value: "prod".to_string(),
                            label: "Production".to_string(),
                        },
                    ],
                    radio: false,
                },
                default: None,
            },
            Field {
                key: "score".to_string(),
                label: "Score".to_string(),
                description: None,
                input: Input::Number {
                    min: Some(1.0),
                    max: Some(5.0),
                    step: Some(1.0),
                },
                default: None,
            },
            Field {
                key: "agree".to_string(),
                label: "I agree".to_string(),
                description: None,
                input: Input::Checkbox,
                default: None,
            },
        ],
        actions: vec![
            Action {
                id: "submit".to_string(),
                label: "Submit".to_string(),
                style: ActionStyle::Primary,
                requires: vec!["name".to_string(), "env".to_string()],
                effect: Effect::UpdateIssue {
                    status: hydra_common::api::v1::projects::StatusKey::try_new("closed").unwrap(),
                    set_feedback_from: None,
                },
            },
            Action {
                id: "skip".to_string(),
                label: "Skip".to_string(),
                style: ActionStyle::Default,
                requires: vec![],
                effect: Effect::RecordOnly,
            },
        ],
    }
}

/// Creates an issue with a form and returns its ID.
async fn create_issue_with_form(
    client: &reqwest::Client,
    base_url: &str,
    form: Form,
) -> anyhow::Result<IssueId> {
    let mut input: hydra_common::api::v1::issues::IssueInput = issue(
        IssueType::Task,
        "issue with form",
        default_user(),
        String::new(),
        status("open"),
        None,
        vec![],
        Vec::new(),
    )
    .into();
    input.form = Some(form);

    let created: UpsertIssueResponse = client
        .post(format!("{base_url}/v1/issues"))
        .json(&UpsertIssueRequest::new(input, None))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(created.issue_id)
}

#[tokio::test]
async fn submit_form_action_valid_submission() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let issue_id = create_issue_with_form(&client, &server.base_url(), test_form()).await?;

    let mut values = HashMap::new();
    values.insert("name".to_string(), json!("Alice"));
    values.insert("env".to_string(), json!("staging"));
    values.insert("score".to_string(), json!(4));

    let resp: SubmitFormResponse = client
        .post(format!(
            "{}/v1/issues/{}/actions",
            server.base_url(),
            issue_id
        ))
        .json(&SubmitFormRequest::new("submit".to_string(), values))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert_eq!(resp.issue_id, issue_id);
    assert_eq!(resp.form_response.action_id, "submit");
    assert_eq!(resp.form_response.values["name"], json!("Alice"));
    assert_eq!(resp.form_response.values["env"], json!("staging"));

    // Verify issue was updated (status should be closed due to UpdateIssue effect)
    let fetched: IssueVersionRecord = client
        .get(format!("{}/v1/issues/{}", server.base_url(), issue_id))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(fetched.issue.status.key.as_str(), "closed");
    // Form should still be present
    assert!(fetched.issue.form.is_some());
    // FormResponse should be stored
    assert!(fetched.issue.form_response.is_some());
    assert_eq!(fetched.issue.form_response.unwrap().action_id, "submit");

    Ok(())
}

#[tokio::test]
async fn submit_form_action_writes_feedback_from_field_atomically() -> anyhow::Result<()> {
    // `Effect::UpdateIssue { set_feedback_from: Some(field) }` writes
    // the named form-field value into `issue.feedback` in the same write
    // that sets `status`. This powers the same-issue review hand-off —
    // the reviewer's `request_changes` action both moves the issue back
    // to in-development and stamps the review comment as feedback so
    // the SWE respawn picks it up.
    let server = spawn_test_server().await?;
    let client = test_client();

    let form = Form {
        prompt: "Reviewer".to_string(),
        fields: vec![Field {
            key: "review_comment".to_string(),
            label: "Comment".to_string(),
            description: None,
            input: Input::Textarea {
                placeholder: None,
                min_length: None,
                max_length: None,
                rows: 4,
            },
            default: None,
        }],
        actions: vec![Action {
            id: "request_changes".to_string(),
            label: "Request changes".to_string(),
            style: ActionStyle::Danger,
            requires: vec!["review_comment".to_string()],
            effect: Effect::UpdateIssue {
                status: hydra_common::api::v1::projects::StatusKey::try_new("in-progress").unwrap(),
                set_feedback_from: Some("review_comment".to_string()),
            },
        }],
    };
    let issue_id = create_issue_with_form(&client, &server.base_url(), form).await?;

    let mut values = HashMap::new();
    values.insert(
        "review_comment".to_string(),
        json!("please address X and Y"),
    );

    let _: SubmitFormResponse = client
        .post(format!(
            "{}/v1/issues/{}/actions",
            server.base_url(),
            issue_id
        ))
        .json(&SubmitFormRequest::new(
            "request_changes".to_string(),
            values,
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let fetched: IssueVersionRecord = client
        .get(format!("{}/v1/issues/{}", server.base_url(), issue_id))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(fetched.issue.status.key.as_str(), "in-progress");
    assert_eq!(
        fetched.issue.feedback,
        Some("please address X and Y".to_string()),
        "set_feedback_from must copy the named field's value into issue.feedback"
    );

    Ok(())
}

#[tokio::test]
async fn submit_form_action_set_feedback_from_rejects_missing_field() -> anyhow::Result<()> {
    // A misconfigured action that names a `set_feedback_from` field but
    // does not list it in `requires` (so the field can legally be absent
    // from a submission) must fail loudly rather than silently stamping
    // an empty feedback string.
    let server = spawn_test_server().await?;
    let client = test_client();

    let form = Form {
        prompt: "Reviewer".to_string(),
        fields: vec![Field {
            key: "review_comment".to_string(),
            label: "Comment".to_string(),
            description: None,
            input: Input::Textarea {
                placeholder: None,
                min_length: None,
                max_length: None,
                rows: 4,
            },
            default: None,
        }],
        // Note: `requires` intentionally empty — this is the misconfiguration.
        actions: vec![Action {
            id: "request_changes".to_string(),
            label: "Request changes".to_string(),
            style: ActionStyle::Danger,
            requires: vec![],
            effect: Effect::UpdateIssue {
                status: hydra_common::api::v1::projects::StatusKey::try_new("in-progress").unwrap(),
                set_feedback_from: Some("review_comment".to_string()),
            },
        }],
    };
    let issue_id = create_issue_with_form(&client, &server.base_url(), form).await?;

    let resp = client
        .post(format!(
            "{}/v1/issues/{}/actions",
            server.base_url(),
            issue_id
        ))
        .json(&SubmitFormRequest::new(
            "request_changes".to_string(),
            HashMap::new(),
        ))
        .send()
        .await?;
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    let fetched: IssueVersionRecord = client
        .get(format!("{}/v1/issues/{}", server.base_url(), issue_id))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(
        fetched.issue.feedback, None,
        "issue.feedback must not be touched when the action fails"
    );

    Ok(())
}

#[tokio::test]
async fn submit_form_action_record_only_does_not_change_status() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let issue_id = create_issue_with_form(&client, &server.base_url(), test_form()).await?;

    let resp: SubmitFormResponse = client
        .post(format!(
            "{}/v1/issues/{}/actions",
            server.base_url(),
            issue_id
        ))
        .json(&SubmitFormRequest::new("skip".to_string(), HashMap::new()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert_eq!(resp.form_response.action_id, "skip");

    // Status should remain open
    let fetched: IssueVersionRecord = client
        .get(format!("{}/v1/issues/{}", server.base_url(), issue_id))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(fetched.issue.status.key.as_str(), "open");

    Ok(())
}

#[tokio::test]
async fn submit_form_action_missing_required_fields() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let issue_id = create_issue_with_form(&client, &server.base_url(), test_form()).await?;

    // Submit without required fields
    let resp = client
        .post(format!(
            "{}/v1/issues/{}/actions",
            server.base_url(),
            issue_id
        ))
        .json(&SubmitFormRequest::new(
            "submit".to_string(),
            HashMap::new(),
        ))
        .send()
        .await?;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: FormValidationError = resp.json().await?;
    assert_eq!(body.error, "validation_failed");
    assert!(body.field_errors.contains_key("name"));
    assert!(body.field_errors.contains_key("env"));

    Ok(())
}

#[tokio::test]
async fn submit_form_action_type_mismatch() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let issue_id = create_issue_with_form(&client, &server.base_url(), test_form()).await?;

    // Provide wrong types: number for name, string for score
    let mut values = HashMap::new();
    values.insert("name".to_string(), json!(42));
    values.insert("env".to_string(), json!("staging"));
    values.insert("score".to_string(), json!("not a number"));

    let resp = client
        .post(format!(
            "{}/v1/issues/{}/actions",
            server.base_url(),
            issue_id
        ))
        .json(&SubmitFormRequest::new("submit".to_string(), values))
        .send()
        .await?;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: FormValidationError = resp.json().await?;
    assert!(body.field_errors.contains_key("name"));
    assert!(body.field_errors.contains_key("score"));

    Ok(())
}

#[tokio::test]
async fn submit_form_action_unknown_keys_rejected() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let issue_id = create_issue_with_form(&client, &server.base_url(), test_form()).await?;

    let mut values = HashMap::new();
    values.insert("name".to_string(), json!("Alice"));
    values.insert("env".to_string(), json!("staging"));
    values.insert("unknown_field".to_string(), json!("bad"));

    let resp = client
        .post(format!(
            "{}/v1/issues/{}/actions",
            server.base_url(),
            issue_id
        ))
        .json(&SubmitFormRequest::new("submit".to_string(), values))
        .send()
        .await?;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: FormValidationError = resp.json().await?;
    assert!(body.field_errors.contains_key("unknown_field"));

    Ok(())
}

#[tokio::test]
async fn submit_form_action_nonexistent_action() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let issue_id = create_issue_with_form(&client, &server.base_url(), test_form()).await?;

    let resp = client
        .post(format!(
            "{}/v1/issues/{}/actions",
            server.base_url(),
            issue_id
        ))
        .json(&SubmitFormRequest::new(
            "nonexistent".to_string(),
            HashMap::new(),
        ))
        .send()
        .await?;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn submit_form_action_no_form_on_issue() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create issue without a form
    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "no form",
                default_user(),
                String::new(),
                status("open"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let resp = client
        .post(format!(
            "{}/v1/issues/{}/actions",
            server.base_url(),
            created.issue_id
        ))
        .json(&SubmitFormRequest::new(
            "submit".to_string(),
            HashMap::new(),
        ))
        .send()
        .await?;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn submit_form_action_select_invalid_option() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let issue_id = create_issue_with_form(&client, &server.base_url(), test_form()).await?;

    let mut values = HashMap::new();
    values.insert("name".to_string(), json!("Alice"));
    values.insert("env".to_string(), json!("invalid_env"));

    let resp = client
        .post(format!(
            "{}/v1/issues/{}/actions",
            server.base_url(),
            issue_id
        ))
        .json(&SubmitFormRequest::new("submit".to_string(), values))
        .send()
        .await?;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: FormValidationError = resp.json().await?;
    assert!(body.field_errors.contains_key("env"));

    Ok(())
}

// ===== Feedback Endpoint Tests =====

#[tokio::test]
async fn submit_feedback_sets_feedback_field() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create an in-progress issue
    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "test feedback",
                default_user(),
                String::new(),
                status("in-progress"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    // Submit feedback
    let resp: IssueVersionRecord = client
        .post(format!(
            "{}/v1/issues/{}/feedback",
            server.base_url(),
            created.issue_id
        ))
        .json(&SubmitFeedbackRequest::new("fix this".to_string()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert_eq!(resp.issue_id, created.issue_id);
    assert_eq!(resp.issue.feedback, Some("fix this".to_string()));
    // Status should remain InProgress (not terminal)
    assert_eq!(resp.issue.status.key.as_str(), "in-progress");

    Ok(())
}

#[tokio::test]
async fn submit_feedback_leaves_closed_status_unchanged() -> anyhow::Result<()> {
    // `submit_feedback` does not mutate status. Callers that want to
    // re-route the issue submit an explicit status transition (typically
    // via a form action), which gives the project's `on_enter` automation
    // a chance to reassign deterministically.
    let server = spawn_test_server().await?;
    let client = test_client();

    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "closed issue",
                default_user(),
                String::new(),
                status("closed"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let resp: IssueVersionRecord = client
        .post(format!(
            "{}/v1/issues/{}/feedback",
            server.base_url(),
            created.issue_id
        ))
        .json(&SubmitFeedbackRequest::new("please reopen".to_string()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert_eq!(resp.issue.feedback, Some("please reopen".to_string()));
    assert_eq!(
        resp.issue.status.key.as_str(),
        "closed",
        "closed issues stay closed when feedback is submitted"
    );

    Ok(())
}

#[tokio::test]
async fn submit_feedback_leaves_failed_status_unchanged() -> anyhow::Result<()> {
    // See `submit_feedback_leaves_closed_status_unchanged`.
    let server = spawn_test_server().await?;
    let client = test_client();

    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "failed issue",
                default_user(),
                String::new(),
                status("failed"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let resp: IssueVersionRecord = client
        .post(format!(
            "{}/v1/issues/{}/feedback",
            server.base_url(),
            created.issue_id
        ))
        .json(&SubmitFeedbackRequest::new("try again".to_string()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert_eq!(resp.issue.feedback, Some("try again".to_string()));
    assert_eq!(
        resp.issue.status.key.as_str(),
        "failed",
        "failed issues stay failed when feedback is submitted"
    );

    Ok(())
}

#[tokio::test]
async fn submit_feedback_nonexistent_issue_returns_404() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let resp = client
        .post(format!(
            "{}/v1/issues/i-nonexistent/feedback",
            server.base_url()
        ))
        .json(&SubmitFeedbackRequest::new("feedback".to_string()))
        .send()
        .await?;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn submit_feedback_deleted_issue_returns_404() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create and delete an issue
    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "to be deleted",
                default_user(),
                String::new(),
                status("open"),
                None,
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    // Delete the issue
    client
        .delete(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .error_for_status()?;

    // Try to submit feedback on deleted issue
    let resp = client
        .post(format!(
            "{}/v1/issues/{}/feedback",
            server.base_url(),
            created.issue_id
        ))
        .json(&SubmitFeedbackRequest::new("feedback".to_string()))
        .send()
        .await?;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn submit_feedback_kills_active_sessions() -> anyhow::Result<()> {
    use crate::{
        domain::{actors::ActorRef, issues::Issue, users::Username},
        job_engine::{JobEngine, JobStatus},
        store::{Session, Status},
        test_utils::{
            MockJobEngine, spawn_test_server_with_state, test_client,
            test_state_with_engine_handles,
        },
    };
    use chrono::Utc;
    use std::sync::Arc;

    let engine = Arc::new(MockJobEngine::new());
    let handles = test_state_with_engine_handles(engine.clone());
    let state = handles.state;
    let store = handles.store.clone();

    // Create an in-progress issue
    let (issue_id, _) = store
        .add_issue(
            Issue {
                issue_type: IssueType::Task,
                title: "test feedback kills sessions".to_string(),
                description: "test".to_string(),
                creator: Username::from("test-creator"),
                progress: String::new(),
                status: status("in-progress"),
                project_id: crate::domain::projects::default_project_id(),
                assignee: None,
                session_settings: Default::default(),
                dependencies: Vec::new(),
                patches: Vec::new(),
                deleted: false,
                form: None,
                form_response: None,
                feedback: None,
            },
            &ActorRef::test(),
        )
        .await?;

    // Helper to create a session linked to this issue
    let make_session = || {
        use crate::domain::sessions::{AgentConfig, SessionMode};
        use crate::routes::sessions::mount_spec_from_create_request;
        Session {
            creator: Username::from("test-creator"),
            spawned_from: Some(issue_id.clone()),
            resumed_from: None,
            agent_config: AgentConfig::default(),
            mount_spec: mount_spec_from_create_request(
                hydra_common::api::v1::sessions::Bundle::None,
                None,
            ),
            image: None,
            env_vars: HashMap::new(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            mode: SessionMode::Headless,
            status: Status::Created,
            last_message: None,
            error: None,
            deleted: false,
            creation_time: None,
            start_time: None,
            end_time: None,
            usage: None,
            proxy_targets: Vec::new(),
        }
    };

    // Session 1: Running (should be killed)
    let (s_running, _) = store
        .add_session(make_session(), Utc::now(), &ActorRef::test())
        .await?;
    state
        .transition_task_to_pending(&s_running, ActorRef::test())
        .await?;
    state
        .transition_task_to_running(&s_running, ActorRef::test())
        .await?;
    engine.insert_job(&s_running, JobStatus::Running).await;

    // Session 2: Pending (should be killed)
    let (s_pending, _) = store
        .add_session(make_session(), Utc::now(), &ActorRef::test())
        .await?;
    state
        .transition_task_to_pending(&s_pending, ActorRef::test())
        .await?;
    engine.insert_job(&s_pending, JobStatus::Pending).await;

    // Session 3: Completed (should NOT be killed)
    let (s_complete, _) = store
        .add_session(make_session(), Utc::now(), &ActorRef::test())
        .await?;
    state
        .transition_task_to_pending(&s_complete, ActorRef::test())
        .await?;
    state
        .transition_task_to_running(&s_complete, ActorRef::test())
        .await?;
    state
        .transition_task_to_completion(&s_complete, Ok(()), None, None, ActorRef::test())
        .await?;
    engine.insert_job(&s_complete, JobStatus::Complete).await;

    // Session 4: Failed (should NOT be killed)
    let (s_failed, _) = store
        .add_session(make_session(), Utc::now(), &ActorRef::test())
        .await?;
    state
        .transition_task_to_pending(&s_failed, ActorRef::test())
        .await?;
    state
        .transition_task_to_running(&s_failed, ActorRef::test())
        .await?;
    state
        .transition_task_to_completion(
            &s_failed,
            Err(crate::domain::task_status::TaskError::JobEngineError {
                reason: "err".to_string(),
            }),
            None,
            None,
            ActorRef::test(),
        )
        .await?;
    engine.insert_job(&s_failed, JobStatus::Failed).await;

    // Session 5: Created (should be killed)
    let (s_created, _) = store
        .add_session(make_session(), Utc::now(), &ActorRef::test())
        .await?;
    engine.insert_job(&s_created, JobStatus::Pending).await;

    let server = spawn_test_server_with_state(state, store).await?;
    let client = test_client();

    // Submit feedback
    client
        .post(format!(
            "{}/v1/issues/{}/feedback",
            server.base_url(),
            issue_id
        ))
        .json(&SubmitFeedbackRequest::new("please fix".to_string()))
        .send()
        .await?
        .error_for_status()?;

    // Active sessions should have been killed (job status -> Failed)
    let running_job = engine.find_job_by_hydra_id(&s_running).await?;
    assert_eq!(
        running_job.status,
        JobStatus::Failed,
        "Running session should have been killed"
    );

    let pending_job = engine.find_job_by_hydra_id(&s_pending).await?;
    assert_eq!(
        pending_job.status,
        JobStatus::Failed,
        "Pending session should have been killed"
    );

    // Terminal sessions should be unchanged
    let complete_job = engine.find_job_by_hydra_id(&s_complete).await?;
    assert_eq!(
        complete_job.status,
        JobStatus::Complete,
        "Completed session should NOT have been killed"
    );

    let failed_job = engine.find_job_by_hydra_id(&s_failed).await?;
    assert_eq!(
        failed_job.status,
        JobStatus::Failed,
        "Already-failed session should NOT have been affected"
    );

    let created_job = engine.find_job_by_hydra_id(&s_created).await?;
    assert_eq!(
        created_job.status,
        JobStatus::Failed,
        "Created session should have been killed"
    );

    Ok(())
}
