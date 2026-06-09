//! HTTP route tests for `/v1/projects` and the `Issue.resolved_status`
//! wire field. Covers PR 3 of the per-project configurable issue
//! statuses design ([[d-druoexk]] §4 / §7).

use crate::{
    domain::{
        issues::{Issue, IssueStatus, IssueType},
        users::Username,
    },
    test_utils::{spawn_test_server, test_client},
};
use hydra_common::api::v1::{
    issues::{IssueVersionRecord, UpsertIssueRequest, UpsertIssueResponse},
    projects::{
        ListProjectsResponse, Project, ProjectKey, ProjectRecord, ProjectStatusesResponse,
        RenameStatusRequest, StatusDefinition, StatusKey, UpsertProjectRequest,
        UpsertProjectResponse,
    },
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
        api_default_user(),
        false,
        0.0,
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
        .json(&UpsertProjectRequest::new(project.clone()))
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
    // The seeded default project is always present, so this round-trip
    // verifies engineering shows up alongside it.
    assert!(
        listed
            .projects
            .iter()
            .any(|p| p.project_id == project_id && p.project.key.as_str() == "engineering"),
        "engineering project must appear in list_projects"
    );

    let mut updated_project = project.clone();
    updated_project.name = "Engineering v2".to_string();
    let update_resp: UpsertProjectResponse = client
        .put(format!("{base}/v1/projects/{project_id}"))
        .json(&UpsertProjectRequest::new(updated_project))
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
        .json(&UpsertProjectRequest::new(sample_project()))
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
        .get(format!(
            "{base}/v1/projects/{}/statuses",
            crate::domain::projects::DEFAULT_PROJECT_ID_STR
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let keys: Vec<&str> = statuses.statuses.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["open", "in-progress", "closed", "dropped", "failed"]);

    // Assert the seeded default-project flag semantics:
    //   closed:  unblocks_parents=true,  unblocks_dependents=true,  cascades_to_children=false
    //   failed:  unblocks_parents=true,  unblocks_dependents=false, cascades_to_children=true
    //   dropped: unblocks_parents=true,  unblocks_dependents=false, cascades_to_children=true
    // (open and in-progress are all-false; not asserted here.)
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
        .json(&UpsertProjectRequest::new(sample_project()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let project_id = create_resp.project_id;

    let mut input: hydra_common::api::v1::issues::IssueInput = Issue::new(
        IssueType::Task,
        "Custom project status".to_string(),
        "test".to_string(),
        default_user(),
        String::new(),
        StatusKey::try_new("backlog").unwrap(),
        crate::domain::projects::default_project_id(),
        None,
        None,
        Vec::new(),
        Vec::new(),
        None,
        None,
        None,
    )
    .into();
    input.project_id = project_id.clone();

    let created: UpsertIssueResponse = client
        .post(format!("{base}/v1/issues"))
        .json(&UpsertIssueRequest::new(input, None))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    // Read back: the response's status field carries the project's
    // `backlog` StatusDefinition inline (label, color, flags).
    let fetched: IssueVersionRecord = client
        .get(format!("{base}/v1/issues/{}", created.issue_id))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let resolved = &fetched.issue.status;
    assert_eq!(resolved.key.as_str(), "backlog");
    assert_eq!(fetched.issue.project_id, project_id);
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
        .json(&UpsertProjectRequest::new(sample_project()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let project_id = create_resp.project_id;

    let mut input: hydra_common::api::v1::issues::IssueInput = Issue::new(
        IssueType::Task,
        "Bogus status".to_string(),
        "test".to_string(),
        default_user(),
        String::new(),
        StatusKey::try_new("not-a-real-status").unwrap(),
        crate::domain::projects::default_project_id(),
        None,
        None,
        Vec::new(),
        Vec::new(),
        None,
        None,
        None,
    )
    .into();
    input.project_id = project_id;

    let resp = client
        .post(format!("{base}/v1/issues"))
        .json(&UpsertIssueRequest::new(input, None))
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
        let input: hydra_common::api::v1::issues::IssueInput = Issue::new(
            IssueType::Task,
            format!("Issue with status {status:?}"),
            "test".to_string(),
            default_user(),
            String::new(),
            status.into(),
            crate::domain::projects::default_project_id(),
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
            .json(&UpsertIssueRequest::new(input, None))
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
        // Issues created via `Issue::new` now persist the seeded
        // default-project id rather than NULL; the response carries the
        // server-resolved `StatusDefinition` inline on `status`.
        assert_eq!(
            fetched.issue.project_id.as_ref(),
            crate::domain::projects::DEFAULT_PROJECT_ID_STR
        );
        let resolved = &fetched.issue.status;
        assert_eq!(resolved.key.as_str(), status.as_str());

        // Assert the seeded default-project flag values:
        //   open/in-progress: unblocks_parents=false, unblocks_dependents=false, cascades_to_children=false
        //   closed:           unblocks_parents=true,  unblocks_dependents=true,  cascades_to_children=false
        //   dropped/failed:   unblocks_parents=true,  unblocks_dependents=false, cascades_to_children=true
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
async fn rename_project_status_route_round_trip() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let create_resp: UpsertProjectResponse = client
        .post(format!("{base}/v1/projects"))
        .json(&UpsertProjectRequest::new(sample_project()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let project_id = create_resp.project_id.clone();

    let mut issue: hydra_common::api::v1::issues::Issue = Issue::new(
        IssueType::Task,
        "backlog item".to_string(),
        "test".to_string(),
        default_user(),
        String::new(),
        StatusKey::try_new("backlog").unwrap(),
        crate::domain::projects::default_project_id(),
        None,
        None,
        Vec::new(),
        Vec::new(),
        None,
        None,
        None,
    )
    .into();
    issue.project_id = project_id.clone();
    let created: UpsertIssueResponse = client
        .post(format!("{base}/v1/issues"))
        .json(&UpsertIssueRequest::new(issue.into(), None))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let rename_resp: UpsertProjectResponse = client
        .post(format!("{base}/v1/projects/{project_id}/statuses/rename"))
        .json(&RenameStatusRequest::new(
            StatusKey::try_new("backlog").unwrap(),
            StatusKey::try_new("triage").unwrap(),
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(rename_resp.project_id, project_id);
    assert_eq!(rename_resp.version, 2);

    let statuses: ProjectStatusesResponse = client
        .get(format!("{base}/v1/projects/{project_id}/statuses"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let keys: Vec<&str> = statuses.statuses.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(
        keys,
        ["triage", "in-development", "in-review", "released"],
        "renamed key must surface in the project's status list"
    );

    let fetched: IssueVersionRecord = client
        .get(format!("{base}/v1/issues/{}", created.issue_id))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(
        fetched.issue.status.as_str(),
        "triage",
        "existing issue must read back with the renamed key"
    );

    Ok(())
}

#[tokio::test]
async fn rename_project_status_to_existing_returns_400() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let create_resp: UpsertProjectResponse = client
        .post(format!("{base}/v1/projects"))
        .json(&UpsertProjectRequest::new(sample_project()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let project_id = create_resp.project_id;

    let resp = client
        .post(format!("{base}/v1/projects/{project_id}/statuses/rename"))
        .json(&RenameStatusRequest::new(
            StatusKey::try_new("backlog").unwrap(),
            StatusKey::try_new("in-development").unwrap(),
        ))
        .send()
        .await?;
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn duplicate_project_key_returns_400() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    client
        .post(format!("{base}/v1/projects"))
        .json(&UpsertProjectRequest::new(sample_project()))
        .send()
        .await?
        .error_for_status()?;

    let resp = client
        .post(format!("{base}/v1/projects"))
        .json(&UpsertProjectRequest::new(sample_project()))
        .send()
        .await?;
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    Ok(())
}
