//! HTTP route tests for `/v1/projects` and the `Issue.resolved_status`
//! wire field. Covers PR 3 of the per-project configurable issue
//! statuses design ([[d-druoexk]] §4 / §7).

use crate::{
    domain::{
        issues::{Issue, IssueStatus, IssueType},
        users::Username,
    },
    routes::projects::{
        ListProjectsResponse, ProjectRecord, ProjectStatusesResponse, UpsertProjectRequest,
        UpsertProjectResponse,
    },
    test_utils::{spawn_test_server, test_client},
};
use hydra_common::api::v1::{
    issues::{IssueVersionRecord, UpsertIssueRequest, UpsertIssueResponse},
    projects::{IconKey, Project, ProjectKey, StatusDefinition, StatusKey},
    users::Username as ApiUsername,
};

fn default_user() -> Username {
    Username::from("creator")
}

fn api_default_user() -> ApiUsername {
    ApiUsername::try_new("creator").unwrap()
}

fn make_status(key: &str, label: &str, color: &str) -> StatusDefinition {
    make_status_with_flags(key, label, color, false, false, false)
}

fn make_status_with_flags(
    key: &str,
    label: &str,
    color: &str,
    unblocks_parents: bool,
    unblocks_dependents: bool,
    cascades_to_children: bool,
) -> StatusDefinition {
    StatusDefinition::new(
        StatusKey::try_new(key).unwrap(),
        label.to_string(),
        IconKey::try_new("circle").unwrap(),
        color.parse().unwrap(),
        unblocks_parents,
        unblocks_dependents,
        cascades_to_children,
        None,
    )
}

fn sample_project() -> Project {
    Project::new(
        ProjectKey::try_new("engineering").unwrap(),
        "Engineering".to_string(),
        vec![
            make_status("backlog", "Backlog", "#3498db"),
            make_status("in-development", "In development", "#f1c40f"),
            make_status_with_flags("in-review", "In review", "#9b59b6", false, false, false),
            make_status_with_flags("released", "Released", "#2ecc71", true, true, false),
        ],
        StatusKey::try_new("backlog").unwrap(),
        api_default_user(),
        false,
    )
}

#[tokio::test]
async fn project_crud_round_trip() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let project = sample_project();
    let create_resp: UpsertProjectResponse = client
        .post(format!("{base}/v1/projects"))
        .json(&UpsertProjectRequest {
            project: project.clone(),
        })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let project_id = create_resp.project_id;
    assert_eq!(create_resp.version, 1);

    let fetched: ProjectRecord = client
        .get(format!("{base}/v1/projects/{project_id}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(fetched.project_id, project_id);
    assert_eq!(fetched.project.key.as_str(), "engineering");
    assert_eq!(fetched.project.statuses.len(), 4);

    let listed: ListProjectsResponse = client
        .get(format!("{base}/v1/projects"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(listed.projects.len(), 1);
    assert_eq!(listed.projects[0].project_id, project_id);

    let mut updated_project = project.clone();
    updated_project.name = "Engineering v2".to_string();
    let update_resp: UpsertProjectResponse = client
        .put(format!("{base}/v1/projects/{project_id}"))
        .json(&UpsertProjectRequest {
            project: updated_project,
        })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(update_resp.version, 2);

    let after_update: ProjectRecord = client
        .get(format!("{base}/v1/projects/{project_id}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(after_update.project.name, "Engineering v2");

    let delete_resp = client
        .delete(format!("{base}/v1/projects/{project_id}"))
        .send()
        .await?
        .error_for_status()?;
    assert!(delete_resp.status().is_success());

    let after_delete = client
        .get(format!("{base}/v1/projects/{project_id}"))
        .send()
        .await?;
    assert_eq!(after_delete.status(), reqwest::StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn project_statuses_route_returns_status_list() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let create_resp: UpsertProjectResponse = client
        .post(format!("{base}/v1/projects"))
        .json(&UpsertProjectRequest {
            project: sample_project(),
        })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let project_id = create_resp.project_id;

    let statuses: ProjectStatusesResponse = client
        .get(format!("{base}/v1/projects/{project_id}/statuses"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(statuses.statuses.len(), 4);
    assert_eq!(statuses.default_status_key, "backlog");
    let keys: Vec<&str> = statuses.statuses.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["backlog", "in-development", "in-review", "released"]);

    Ok(())
}

#[tokio::test]
async fn default_project_statuses_route_returns_legacy_status_list() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let statuses: ProjectStatusesResponse = client
        .get(format!("{base}/v1/projects/default/statuses"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert_eq!(statuses.default_status_key, "open");
    let keys: Vec<&str> = statuses.statuses.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["open", "in-progress", "closed", "dropped", "failed"]);

    // Flag semantics from /designs/per-project-issue-statuses.md §4
    // "Default-project synthesis" table.
    let by_key: std::collections::HashMap<&str, &StatusDefinition> = statuses
        .statuses
        .iter()
        .map(|s| (s.key.as_str(), s))
        .collect();

    let closed = by_key["closed"];
    assert!(closed.unblocks_parents);
    assert!(closed.unblocks_dependents);
    assert!(!closed.cascades_to_children);

    let failed = by_key["failed"];
    assert!(failed.unblocks_parents);
    assert!(!failed.unblocks_dependents);
    assert!(failed.cascades_to_children);

    Ok(())
}

#[tokio::test]
async fn issue_with_project_id_and_custom_status_succeeds() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let create_resp: UpsertProjectResponse = client
        .post(format!("{base}/v1/projects"))
        .json(&UpsertProjectRequest {
            project: sample_project(),
        })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let project_id = create_resp.project_id;

    let mut issue: hydra_common::api::v1::issues::Issue = Issue::new(
        IssueType::Task,
        "Custom project status".to_string(),
        "test".to_string(),
        default_user(),
        String::new(),
        StatusKey::try_new("backlog").unwrap(),
        None,
        None,
        Vec::new(),
        Vec::new(),
        None,
        None,
        None,
    )
    .into();
    issue.project_id = Some(project_id.clone());

    let created: UpsertIssueResponse = client
        .post(format!("{base}/v1/issues"))
        .json(&UpsertIssueRequest::new(issue, None))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    // Read back: resolved_status must reflect the project's `backlog`
    // StatusDefinition, not a default-project entry.
    let fetched: IssueVersionRecord = client
        .get(format!("{base}/v1/issues/{}", created.issue_id))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(fetched.issue.status.as_str(), "backlog");
    assert_eq!(fetched.issue.project_id.as_ref(), Some(&project_id));
    let resolved = fetched
        .issue
        .resolved_status
        .as_ref()
        .expect("resolved_status must be populated on every response");
    assert_eq!(resolved.key.as_str(), "backlog");
    assert_eq!(resolved.label, "Backlog");
    assert!(!resolved.unblocks_parents);

    Ok(())
}

#[tokio::test]
async fn issue_with_unknown_status_key_returns_400() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let create_resp: UpsertProjectResponse = client
        .post(format!("{base}/v1/projects"))
        .json(&UpsertProjectRequest {
            project: sample_project(),
        })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let project_id = create_resp.project_id;

    let mut issue: hydra_common::api::v1::issues::Issue = Issue::new(
        IssueType::Task,
        "Bogus status".to_string(),
        "test".to_string(),
        default_user(),
        String::new(),
        StatusKey::try_new("not-a-real-status").unwrap(),
        None,
        None,
        Vec::new(),
        Vec::new(),
        None,
        None,
        None,
    )
    .into();
    issue.project_id = Some(project_id);

    let resp = client
        .post(format!("{base}/v1/issues"))
        .json(&UpsertIssueRequest::new(issue, None))
        .send()
        .await?;
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn default_project_issue_includes_resolved_status_on_every_legacy_key() -> anyhow::Result<()>
{
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    for status in [
        IssueStatus::Open,
        IssueStatus::InProgress,
        IssueStatus::Closed,
        IssueStatus::Dropped,
        IssueStatus::Failed,
    ] {
        let issue: hydra_common::api::v1::issues::Issue = Issue::new(
            IssueType::Task,
            format!("Issue with status {status:?}"),
            "test".to_string(),
            default_user(),
            String::new(),
            status.into(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        )
        .into();

        let created: UpsertIssueResponse = client
            .post(format!("{base}/v1/issues"))
            .json(&UpsertIssueRequest::new(issue, None))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let fetched: IssueVersionRecord = client
            .get(format!("{base}/v1/issues/{}", created.issue_id))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert!(fetched.issue.project_id.is_none());
        let resolved = fetched
            .issue
            .resolved_status
            .as_ref()
            .expect("resolved_status must be populated for default-project issues");
        assert_eq!(resolved.key.as_str(), status.as_str());

        // Flag values per design §4 table.
        match status {
            IssueStatus::Open | IssueStatus::InProgress => {
                assert!(!resolved.unblocks_parents);
                assert!(!resolved.unblocks_dependents);
                assert!(!resolved.cascades_to_children);
            }
            IssueStatus::Closed => {
                assert!(resolved.unblocks_parents);
                assert!(resolved.unblocks_dependents);
                assert!(!resolved.cascades_to_children);
            }
            IssueStatus::Dropped | IssueStatus::Failed => {
                assert!(resolved.unblocks_parents);
                assert!(!resolved.unblocks_dependents);
                assert!(resolved.cascades_to_children);
            }
        }
    }

    Ok(())
}

#[tokio::test]
async fn duplicate_project_key_returns_400() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    client
        .post(format!("{base}/v1/projects"))
        .json(&UpsertProjectRequest {
            project: sample_project(),
        })
        .send()
        .await?
        .error_for_status()?;

    let resp = client
        .post(format!("{base}/v1/projects"))
        .json(&UpsertProjectRequest {
            project: sample_project(),
        })
        .send()
        .await?;
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    Ok(())
}
