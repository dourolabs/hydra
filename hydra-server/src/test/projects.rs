//! HTTP route tests for `/v1/projects` and the `Issue.resolved_status`
//! wire field. Post-cutover the wire shape splits project-level CRUD
//! from per-status CRUD; these tests exercise both surfaces against
//! a live `spawn_test_server`.

use crate::{
    domain::{
        issues::{Issue, IssueType},
        users::Username,
    },
    test_utils::{spawn_test_server, test_client},
};
use hydra_common::api::v1::{
    issues::{IssueVersionRecord, UpsertIssueRequest, UpsertIssueResponse},
    projects::{
        ListProjectsResponse, ProjectKey, ProjectRecord, ProjectStatusesResponse, StatusDefinition,
        StatusKey, UpsertProjectRequest, UpsertProjectResponse, UpsertProjectStatusResponse,
    },
};
use hydra_common::test_utils::status::status;

fn default_user() -> Username {
    Username::from("creator")
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

fn engineering_upsert() -> UpsertProjectRequest {
    UpsertProjectRequest::new(
        ProjectKey::try_new("engineering").unwrap(),
        "Engineering".to_string(),
    )
}

fn engineering_statuses() -> Vec<StatusDefinition> {
    vec![
        make_status("backlog", "Backlog", "#3498db"),
        make_status("in-development", "In development", "#f1c40f"),
        make_status_with_flags("in-review", "In review", "#9b59b6", false, false, false),
        make_status_with_flags("released", "Released", "#2ecc71", true, true, false),
    ]
}

/// Test helper: create the engineering project and add every status
/// from `engineering_statuses()`. Returns the project id.
async fn setup_engineering_project(
    client: &reqwest::Client,
    base: &str,
) -> anyhow::Result<hydra_common::ProjectId> {
    let create_resp: UpsertProjectResponse = client
        .post(format!("{base}/v1/projects"))
        .json(&engineering_upsert())
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let project_id = create_resp.project_id;
    for status in engineering_statuses() {
        client
            .post(format!("{base}/v1/projects/{project_id}/statuses"))
            .json(&status)
            .send()
            .await?
            .error_for_status()?;
    }
    Ok(project_id)
}

#[tokio::test]
async fn project_crud_round_trip() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let create_resp: UpsertProjectResponse = client
        .post(format!("{base}/v1/projects"))
        .json(&engineering_upsert())
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let project_id = create_resp.project_id;
    assert_eq!(create_resp.version, 1);

    // The new project starts with zero statuses; adding one via the
    // per-status route surfaces it on the next GET.
    let backlog = make_status("backlog", "Backlog", "#3498db");
    let add_resp: UpsertProjectStatusResponse = client
        .post(format!("{base}/v1/projects/{project_id}/statuses"))
        .json(&backlog)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(add_resp.project_id, project_id);
    assert_eq!(add_resp.status.key.as_str(), "backlog");

    let fetched: ProjectRecord = client
        .get(format!("{base}/v1/projects/{project_id}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(fetched.project_id, project_id);
    assert_eq!(fetched.project.key.as_str(), "engineering");
    assert_eq!(fetched.project.statuses.len(), 1);
    assert_eq!(fetched.project.statuses[0].key.as_str(), "backlog");

    let listed: ListProjectsResponse = client
        .get(format!("{base}/v1/projects"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(
        listed
            .projects
            .iter()
            .any(|p| p.project_id == project_id && p.project.key.as_str() == "engineering"),
        "engineering project must appear in list_projects"
    );

    let mut updated = engineering_upsert();
    updated.name = "Engineering v2".to_string();
    let update_resp: UpsertProjectResponse = client
        .put(format!("{base}/v1/projects/{project_id}"))
        .json(&updated)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    // 1 (add_project) + 1 (add_status) + 1 (update_project) = 3
    assert_eq!(update_resp.version, 3);

    let after_update: ProjectRecord = client
        .get(format!("{base}/v1/projects/{project_id}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(after_update.project.name, "Engineering v2");
    // Statuses are preserved across project-level updates.
    assert_eq!(after_update.project.statuses.len(), 1);

    let archive_resp = client
        .post(format!("{base}/v1/projects/{project_id}/archive"))
        .send()
        .await?
        .error_for_status()?;
    assert!(archive_resp.status().is_success());

    let after_archive = client
        .get(format!("{base}/v1/projects/{project_id}"))
        .send()
        .await?;
    assert_eq!(after_archive.status(), reqwest::StatusCode::NOT_FOUND);

    Ok(())
}

/// PR-A acceptance: `Project.session_settings` round-trips through the
/// HTTP layer (`POST /v1/projects` → `GET /v1/projects/:id` →
/// `PUT /v1/projects/:id` → `GET`). Covers the wire shape (no
/// `session_settings` key on default), the create path's plumbing, and
/// the update path's preservation/replacement semantics.
#[tokio::test]
async fn project_session_settings_round_trip_through_http() -> anyhow::Result<()> {
    use hydra_common::api::v1::issues::SessionSettings;

    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let mut session_settings = SessionSettings::default();
    session_settings.image = Some("hydra-worker:project".to_string());
    session_settings.cpu_limit = Some("500m".to_string());
    let mut create_req = engineering_upsert();
    create_req.session_settings = session_settings.clone();

    let create_resp: UpsertProjectResponse = client
        .post(format!("{base}/v1/projects"))
        .json(&create_req)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let project_id = create_resp.project_id;

    let fetched: ProjectRecord = client
        .get(format!("{base}/v1/projects/{project_id}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(
        fetched.project.session_settings, session_settings,
        "create-time session_settings must round-trip through GET"
    );

    // Updating with the existing settings keeps them; switching to default wipes them.
    let mut update_req = engineering_upsert();
    update_req.session_settings = SessionSettings::default();
    let _: UpsertProjectResponse = client
        .put(format!("{base}/v1/projects/{project_id}"))
        .json(&update_req)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let after_update: ProjectRecord = client
        .get(format!("{base}/v1/projects/{project_id}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(
        SessionSettings::is_default(&after_update.project.session_settings),
        "PUT with default session_settings must clear the project-level overrides"
    );

    Ok(())
}

#[tokio::test]
async fn project_statuses_route_returns_status_list() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let project_id = setup_engineering_project(&client, &base).await?;

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
async fn create_status_duplicate_key_returns_400() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let project_id = setup_engineering_project(&client, &base).await?;

    let resp = client
        .post(format!("{base}/v1/projects/{project_id}/statuses"))
        .json(&make_status("backlog", "Backlog Again", "#3498db"))
        .send()
        .await?;
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn update_status_in_place_round_trip() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let project_id = setup_engineering_project(&client, &base).await?;

    let mut updated = make_status("backlog", "Renamed label", "#aabbcc");
    updated.position = 250.0;
    let resp: UpsertProjectStatusResponse = client
        .put(format!("{base}/v1/projects/{project_id}/statuses/backlog"))
        .json(&updated)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(resp.status.label, "Renamed label");
    assert_eq!(resp.status.position, 250.0);

    let statuses: ProjectStatusesResponse = client
        .get(format!("{base}/v1/projects/{project_id}/statuses"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let backlog = statuses
        .statuses
        .iter()
        .find(|s| s.key.as_str() == "backlog")
        .expect("backlog still present");
    assert_eq!(backlog.label, "Renamed label");
    assert_eq!(backlog.position, 250.0);

    Ok(())
}

#[tokio::test]
async fn update_status_rename_route_round_trip() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let project_id = setup_engineering_project(&client, &base).await?;

    // Create an issue against `backlog` so we can assert the rename
    // doesn't orphan it.
    let mut input: hydra_common::api::v1::issues::IssueInput = Issue::new(
        IssueType::Task,
        "backlog item".to_string(),
        "test".to_string(),
        default_user(),
        StatusKey::try_new("backlog").unwrap(),
        crate::domain::projects::default_project_id(),
        None,
        None,
        Vec::new(),
        Vec::new(),
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

    let mut renamed = make_status("triage", "Backlog", "#3498db");
    renamed.key = StatusKey::try_new("triage").unwrap();
    let rename_resp: UpsertProjectStatusResponse = client
        .put(format!("{base}/v1/projects/{project_id}/statuses/backlog"))
        .json(&renamed)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(rename_resp.project_id, project_id);
    assert_eq!(rename_resp.status.key.as_str(), "triage");

    let statuses: ProjectStatusesResponse = client
        .get(format!("{base}/v1/projects/{project_id}/statuses"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let keys: Vec<&str> = statuses.statuses.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["triage", "in-development", "in-review", "released"]);

    let fetched: IssueVersionRecord = client
        .get(format!("{base}/v1/issues/{}", created.issue_id))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(
        fetched.issue.status.key.as_str(),
        "triage",
        "existing issue must read back with the renamed key"
    );

    Ok(())
}

/// `StatusOnEnter::validate` rejects an `on_enter` body that sets both
/// `assign_to` and `clear_assignee`. The route handlers must surface
/// the rejection as a 400 rather than persisting a contradictory
/// configuration that the automation can't honour.
#[tokio::test]
async fn create_status_with_assign_to_and_clear_assignee_returns_400() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let project_id = setup_engineering_project(&client, &base).await?;

    let body = serde_json::json!({
        "key": "abandoned",
        "label": "Abandoned",
        "color": "#cccccc",
        "unblocks_parents": false,
        "unblocks_dependents": false,
        "cascades_to_children": false,
        "on_enter": {
            "assign_to": { "User": { "name": "creator" } },
            "clear_assignee": true,
        },
    });
    let resp = client
        .post(format!("{base}/v1/projects/{project_id}/statuses"))
        .json(&body)
        .send()
        .await?;
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    let text = resp.text().await?;
    assert!(
        text.contains("assign_to") && text.contains("clear_assignee"),
        "400 body must name both fields; got: {text}"
    );

    Ok(())
}

#[tokio::test]
async fn update_status_with_assign_to_and_clear_assignee_returns_400() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let project_id = setup_engineering_project(&client, &base).await?;

    let body = serde_json::json!({
        "key": "backlog",
        "label": "Backlog",
        "color": "#3498db",
        "unblocks_parents": false,
        "unblocks_dependents": false,
        "cascades_to_children": false,
        "on_enter": {
            "assign_to": { "User": { "name": "creator" } },
            "clear_assignee": true,
        },
    });
    let resp = client
        .put(format!("{base}/v1/projects/{project_id}/statuses/backlog"))
        .json(&body)
        .send()
        .await?;
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn update_status_rename_to_existing_returns_400() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let project_id = setup_engineering_project(&client, &base).await?;

    let mut collide = make_status("in-development", "Backlog", "#3498db");
    collide.key = StatusKey::try_new("in-development").unwrap();
    let resp = client
        .put(format!("{base}/v1/projects/{project_id}/statuses/backlog"))
        .json(&collide)
        .send()
        .await?;
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn archive_status_route_round_trip() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let project_id = setup_engineering_project(&client, &base).await?;

    // Archive `released`, which has no issues against it — succeeds. The
    // status row stays in the project's `statuses` list (archived flag
    // flipped in place) so reads continue to surface it.
    let resp = client
        .post(format!(
            "{base}/v1/projects/{project_id}/statuses/released/archive"
        ))
        .send()
        .await?
        .error_for_status()?;
    assert!(resp.status().is_success());

    let statuses: ProjectStatusesResponse = client
        .get(format!("{base}/v1/projects/{project_id}/statuses"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let keys: Vec<&str> = statuses.statuses.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["backlog", "in-development", "in-review", "released"]);
    let released = statuses
        .statuses
        .iter()
        .find(|s| s.key.as_str() == "released")
        .expect("released status still present");
    assert!(
        released.archived,
        "archived flag must be true after archive_status"
    );

    Ok(())
}

#[tokio::test]
async fn archive_status_with_active_issue_cascade_archives_it() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let project_id = setup_engineering_project(&client, &base).await?;

    // Create an issue at `backlog`, then archive that status. The
    // cascade-archive flow flips `issue.archived = true` for the
    // active issue — no 400, no FK violation.
    let mut input: hydra_common::api::v1::issues::IssueInput = Issue::new(
        IssueType::Task,
        "backlog item".to_string(),
        "test".to_string(),
        default_user(),
        StatusKey::try_new("backlog").unwrap(),
        crate::domain::projects::default_project_id(),
        None,
        None,
        Vec::new(),
        Vec::new(),
        None,
        None,
    )
    .into();
    input.project_id = project_id.clone();
    let created: hydra_common::api::v1::issues::UpsertIssueResponse = client
        .post(format!("{base}/v1/issues"))
        .json(&UpsertIssueRequest::new(input, None))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let issue_id = created.issue_id;

    let resp = client
        .post(format!(
            "{base}/v1/projects/{project_id}/statuses/backlog/archive"
        ))
        .send()
        .await?;
    assert!(
        resp.status().is_success(),
        "archive_status must cascade, not 400"
    );

    let listed: hydra_common::api::v1::issues::ListIssuesResponse = client
        .get(format!("{base}/v1/issues?include_archived=true"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let cascaded = listed
        .issues
        .iter()
        .find(|i| i.issue_id == issue_id)
        .expect("cascaded issue still in the list with include_archived=true");
    assert!(
        cascaded.issue.archived,
        "cascade must flip issue.archived = true"
    );

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
    let project_id = setup_engineering_project(&client, &base).await?;

    let mut input: hydra_common::api::v1::issues::IssueInput = Issue::new(
        IssueType::Task,
        "Custom project status".to_string(),
        "test".to_string(),
        default_user(),
        StatusKey::try_new("backlog").unwrap(),
        crate::domain::projects::default_project_id(),
        None,
        None,
        Vec::new(),
        Vec::new(),
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
    let project_id = setup_engineering_project(&client, &base).await?;

    let mut input: hydra_common::api::v1::issues::IssueInput = Issue::new(
        IssueType::Task,
        "Bogus status".to_string(),
        "test".to_string(),
        default_user(),
        StatusKey::try_new("not-a-real-status").unwrap(),
        crate::domain::projects::default_project_id(),
        None,
        None,
        Vec::new(),
        Vec::new(),
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

    for status_slug in ["open", "in-progress", "closed", "dropped", "failed"] {
        let input: hydra_common::api::v1::issues::IssueInput = Issue::new(
            IssueType::Task,
            format!("Issue with status {status_slug}"),
            "test".to_string(),
            default_user(),
            status(status_slug),
            crate::domain::projects::default_project_id(),
            None,
            None,
            Vec::new(),
            Vec::new(),
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
        assert_eq!(
            fetched.issue.project_id.as_ref(),
            crate::domain::projects::DEFAULT_PROJECT_ID_STR
        );
        let resolved = &fetched.issue.status;
        assert_eq!(resolved.key.as_str(), status_slug);

        match status_slug {
            "open" | "in-progress" => {
                assert!(!resolved.unblocks_parents);
                assert!(!resolved.unblocks_dependents);
                assert!(!resolved.cascades_to_children);
            }
            "closed" => {
                assert!(resolved.unblocks_parents);
                assert!(resolved.unblocks_dependents);
                assert!(!resolved.cascades_to_children);
            }
            "dropped" | "failed" => {
                assert!(resolved.unblocks_parents);
                assert!(!resolved.unblocks_dependents);
                assert!(resolved.cascades_to_children);
            }
            _ => unreachable!(),
        }
    }

    Ok(())
}

#[tokio::test]
async fn project_routes_accept_key_alongside_id() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();
    let project_id = setup_engineering_project(&client, &base).await?;

    // GET by key
    let fetched_by_key: ProjectRecord = client
        .get(format!("{base}/v1/projects/engineering"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(fetched_by_key.project_id, project_id);
    assert_eq!(fetched_by_key.project.key.as_str(), "engineering");

    // GET /statuses by key
    let statuses_by_key: ProjectStatusesResponse = client
        .get(format!("{base}/v1/projects/engineering/statuses"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let keys: Vec<&str> = statuses_by_key
        .statuses
        .iter()
        .map(|s| s.key.as_str())
        .collect();
    assert_eq!(keys, ["backlog", "in-development", "in-review", "released"]);

    // Rename by key — PUT .../statuses/backlog with body.key = "triage".
    let mut renamed = make_status("triage", "Backlog", "#3498db");
    renamed.key = StatusKey::try_new("triage").unwrap();
    let rename_resp: UpsertProjectStatusResponse = client
        .put(format!("{base}/v1/projects/engineering/statuses/backlog"))
        .json(&renamed)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(rename_resp.project_id, project_id);

    let statuses: ProjectStatusesResponse = client
        .get(format!("{base}/v1/projects/{project_id}/statuses"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let keys: Vec<&str> = statuses.statuses.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["triage", "in-development", "in-review", "released"]);

    // PUT by key — project-level update.
    let mut updated = engineering_upsert();
    updated.name = "Engineering v3".to_string();
    let update_resp: UpsertProjectResponse = client
        .put(format!("{base}/v1/projects/engineering"))
        .json(&updated)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(update_resp.project_id, project_id);
    let after_update: ProjectRecord = client
        .get(format!("{base}/v1/projects/engineering"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(after_update.project.name, "Engineering v3");

    // Archive by key.
    let archive_resp: UpsertProjectResponse = client
        .post(format!("{base}/v1/projects/engineering/archive"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(archive_resp.project_id, project_id);

    let resp = client
        .get(format!("{base}/v1/projects/engineering"))
        .send()
        .await?;
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn project_routes_404_quotes_key_in_body() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let resp = client
        .get(format!("{base}/v1/projects/not-a-real-project"))
        .send()
        .await?;
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    let body = resp.text().await?;
    assert!(
        body.contains("not-a-real-project"),
        "404 body must quote the missing key; got: {body}"
    );

    Ok(())
}

#[tokio::test]
async fn project_routes_404_for_id_shape_with_no_match() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    let resp = client
        .get(format!("{base}/v1/projects/j-zzzzzz"))
        .send()
        .await?;
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    let body = resp.text().await?;
    assert!(
        body.contains("j-zzzzzz"),
        "404 body must quote the missing id; got: {body}"
    );

    Ok(())
}

#[tokio::test]
async fn default_project_statuses_resolves_via_key() -> anyhow::Result<()> {
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
    let keys: Vec<&str> = statuses.statuses.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["open", "in-progress", "closed", "dropped", "failed"]);
    Ok(())
}

#[tokio::test]
async fn duplicate_project_key_returns_400() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    client
        .post(format!("{base}/v1/projects"))
        .json(&engineering_upsert())
        .send()
        .await?
        .error_for_status()?;

    let resp = client
        .post(format!("{base}/v1/projects"))
        .json(&engineering_upsert())
        .send()
        .await?;
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    Ok(())
}
