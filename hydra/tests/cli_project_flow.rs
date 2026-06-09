mod harness;

use anyhow::Result;
use hydra_common::api::v1::projects::{
    Project, ProjectKey, ProjectRef, StatusDefinition, StatusKey, UpsertProjectRequest,
};
use std::io::Write;
use tempfile::NamedTempFile;

fn write_body_file(body: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().expect("create body file");
    file.write_all(body.as_bytes()).expect("write body file");
    file
}

const ENGINEERING_BODY: &str = r##"{
    "statuses": [
        {
            "key": "inbox",
            "label": "Inbox",
            "color": "#aabbcc",
            "unblocks_parents": false,
            "unblocks_dependents": false,
            "cascades_to_children": false
        },
        {
            "key": "backlog",
            "label": "Backlog",
            "color": "#1199ee",
            "unblocks_parents": false,
            "unblocks_dependents": false,
            "cascades_to_children": false
        },
        {
            "key": "released",
            "label": "Released",
            "color": "#22aa44",
            "unblocks_parents": true,
            "unblocks_dependents": true,
            "cascades_to_children": false
        }
    ]
}"##;

#[tokio::test]
async fn cli_projects_crud_round_trip() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/cli-projects")
        .build()
        .await?;
    let user = harness.default_user();
    let body_file = write_body_file(ENGINEERING_BODY);

    // Create a project via the CLI.
    user.cli(&[
        "projects",
        "create",
        "--key",
        "engineering",
        "--name",
        "Engineering",
        "--body-file",
        body_file.path().to_str().expect("body path utf-8"),
    ])
    .await?;

    // List should show the project.
    let listed = user
        .client()
        .list_projects()
        .await?
        .projects
        .into_iter()
        .find(|p| p.project.key.as_str() == "engineering")
        .expect("engineering project listed");
    let project_id = listed.project_id.clone();

    // Get by id round-trips the body.
    let project_ref = ProjectRef::Id(project_id.clone());
    let fetched = user.client().get_project(&project_ref).await?;
    assert_eq!(fetched.project.key.as_str(), "engineering");
    assert_eq!(fetched.project.statuses.len(), 3);

    // Statuses endpoint via CLI.
    let statuses_out = user
        .cli(&[
            "--output-format",
            "jsonl",
            "projects",
            "statuses",
            project_id.as_ref(),
        ])
        .await?;
    assert!(
        statuses_out.stdout.contains("inbox") && statuses_out.stdout.contains("released"),
        "statuses output missing keys: {}",
        statuses_out.stdout
    );

    // Update via CLI (rename only).
    user.cli(&[
        "projects",
        "update",
        project_id.as_ref(),
        "--name",
        "Engineering Org",
    ])
    .await?;
    let after_rename = user.client().get_project(&project_ref).await?;
    assert_eq!(after_rename.project.name, "Engineering Org");
    assert_eq!(
        after_rename.project.statuses.len(),
        3,
        "rename should preserve statuses"
    );

    // Delete via CLI; soft-delete leaves the row visible to subsequent
    // operations through the API but `list_projects` should not include it.
    user.cli(&["projects", "delete", project_id.as_ref()])
        .await?;
    let listed_after = user.client().list_projects().await?.projects;
    assert!(
        !listed_after.iter().any(|p| p.project_id == project_id),
        "deleted project should not appear in list"
    );

    Ok(())
}

#[tokio::test]
async fn cli_issues_accepts_custom_status_with_project() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/cli-projects-status")
        .build()
        .await?;
    let user = harness.default_user();

    // Create a project that defines a custom `backlog` status, via the API
    // directly so the test focuses on the CLI's `--status` plumbing.
    let project = Project::new(
        ProjectKey::try_new("engineering").unwrap(),
        "Engineering".into(),
        vec![
            StatusDefinition::new(
                StatusKey::try_new("inbox").unwrap(),
                "Inbox".into(),
                "#aabbcc".parse().unwrap(),
                false,
                false,
                false,
                None,
            ),
            StatusDefinition::new(
                StatusKey::try_new("backlog").unwrap(),
                "Backlog".into(),
                "#1199ee".parse().unwrap(),
                false,
                false,
                false,
                None,
            ),
        ],
        hydra_common::api::v1::users::Username::try_new(user.name()).unwrap(),
        false,
        0.0,
    );
    user.client()
        .create_project(&UpsertProjectRequest::new(project))
        .await?;

    // CLI accepts a custom status string when the project recognises it.
    user.cli(&[
        "issues",
        "create",
        "--type",
        "task",
        "--status",
        "backlog",
        "--project",
        "engineering",
        "backlog-status-issue",
    ])
    .await?;

    // CLI accepts the legacy `open` status against the default project (no
    // `--project` flag).
    user.cli(&[
        "issues",
        "create",
        "--type",
        "task",
        "--status",
        "open",
        "default-status-issue",
    ])
    .await?;

    // CLI rejects an unknown status string against the chosen project — the
    // server returns 400 and the CLI surfaces it as a non-zero exit.
    let failure = user
        .cli_expect_failure(&[
            "issues",
            "create",
            "--type",
            "task",
            "--status",
            "not-a-real-status",
            "--project",
            "engineering",
            "bad-status-issue",
        ])
        .await?;
    assert!(
        failure.stderr.contains("not-a-real-status") || failure.stderr.contains("400"),
        "expected error to mention the bad status, got: {}",
        failure.stderr
    );

    Ok(())
}
