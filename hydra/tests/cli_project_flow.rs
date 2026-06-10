mod harness;

use anyhow::Result;
use hydra_common::api::v1::projects::{
    ProjectKey, ProjectRef, StatusDefinition, StatusKey, UpsertProjectRequest,
};
use std::io::Write;
use tempfile::NamedTempFile;

fn write_body_file(body: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().expect("create body file");
    file.write_all(body.as_bytes()).expect("write body file");
    file
}

const INBOX_BODY: &str = r##"{
    "key": "inbox",
    "label": "Inbox",
    "color": "#aabbcc",
    "unblocks_parents": false,
    "unblocks_dependents": false,
    "cascades_to_children": false
}"##;

const BACKLOG_BODY: &str = r##"{
    "key": "backlog",
    "label": "Backlog",
    "color": "#1199ee",
    "unblocks_parents": false,
    "unblocks_dependents": false,
    "cascades_to_children": false
}"##;

const RELEASED_BODY: &str = r##"{
    "key": "released",
    "label": "Released",
    "color": "#22aa44",
    "unblocks_parents": true,
    "unblocks_dependents": true,
    "cascades_to_children": false
}"##;

#[tokio::test]
async fn cli_projects_crud_round_trip() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/cli-projects")
        .build()
        .await?;
    let user = harness.default_user();
    let inbox_file = write_body_file(INBOX_BODY);
    let backlog_file = write_body_file(BACKLOG_BODY);
    let released_file = write_body_file(RELEASED_BODY);

    // Create a project via the CLI (project-level fields only post-cutover).
    user.cli(&[
        "projects",
        "create",
        "--key",
        "engineering",
        "--name",
        "Engineering",
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

    // Add statuses one at a time via `projects status create --body-file`.
    for body in [&inbox_file, &backlog_file, &released_file] {
        user.cli(&[
            "projects",
            "status",
            "create",
            project_id.as_ref(),
            "--body-file",
            body.path().to_str().expect("body path utf-8"),
        ])
        .await?;
    }

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
async fn cli_projects_status_create_with_direct_flags() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/cli-projects-direct-create")
        .build()
        .await?;
    let user = harness.default_user();

    // Create a project, then add a status using the new direct flags.
    user.cli(&[
        "projects",
        "create",
        "--key",
        "engineering",
        "--name",
        "Engineering",
    ])
    .await?;
    user.cli(&[
        "projects",
        "status",
        "create",
        "engineering",
        "--key",
        "review",
        "--label",
        "In Review",
        "--color",
        "#abcdef",
        "--unblocks-parents",
        "--position",
        "3.5",
        "--auto-archive-after-seconds",
        "1209600",
        "--prompt-path",
        "/projects/engineering/statuses/review.md",
        "--on-enter-assign-to",
        "agents/swe",
        "--on-enter-teardown-work",
    ])
    .await?;

    let fetched = user
        .client()
        .get_project(&ProjectRef::Key(
            ProjectKey::try_new("engineering").unwrap(),
        ))
        .await?;
    let review = fetched
        .project
        .statuses
        .iter()
        .find(|s| s.key == StatusKey::try_new("review").unwrap())
        .expect("review status created");
    assert_eq!(review.label, "In Review");
    assert_eq!(review.color.as_ref(), "#abcdef");
    assert!(review.unblocks_parents);
    assert!(!review.unblocks_dependents);
    assert_eq!(review.position, 3.5);
    assert_eq!(review.auto_archive_after_seconds, Some(1_209_600));
    assert_eq!(
        review.prompt_path.as_deref(),
        Some("/projects/engineering/statuses/review.md"),
    );
    let on_enter = review.on_enter.as_ref().expect("on_enter set");
    match on_enter.assign_to.as_ref().expect("assign_to set") {
        hydra_common::principal::Principal::Agent { name } => {
            assert_eq!(name.as_str(), "swe");
        }
        other => panic!("expected agents/swe, got {other:?}"),
    }
    assert!(on_enter.teardown_work);
    assert!(!on_enter.clear_assignee);

    // Update only the label via direct flags — other fields must
    // round-trip unchanged.
    user.cli(&[
        "projects",
        "status",
        "update",
        "engineering",
        "review",
        "--label",
        "Reviewed",
    ])
    .await?;
    let after_update = user
        .client()
        .get_project(&ProjectRef::Key(
            ProjectKey::try_new("engineering").unwrap(),
        ))
        .await?;
    let review_after = after_update
        .project
        .statuses
        .iter()
        .find(|s| s.key == StatusKey::try_new("review").unwrap())
        .expect("review status still present");
    assert_eq!(review_after.label, "Reviewed");
    assert!(review_after.unblocks_parents);
    assert_eq!(review_after.position, 3.5);
    assert_eq!(review_after.auto_archive_after_seconds, Some(1_209_600));
    assert_eq!(review_after.on_enter, review.on_enter);

    // `update` with no flag should be rejected with a clear error.
    let failure = user
        .cli_expect_failure(&["projects", "status", "update", "engineering", "review"])
        .await?;
    assert!(
        failure.stderr.contains("no updates specified"),
        "expected 'no updates specified', got: {}",
        failure.stderr
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
    let upsert = UpsertProjectRequest::new(
        ProjectKey::try_new("engineering").unwrap(),
        "Engineering".into(),
    );
    let create_resp = user.client().create_project(&upsert).await?;
    let project_ref = ProjectRef::Id(create_resp.project_id.clone());
    for status in [
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
    ] {
        user.client()
            .create_project_status(&project_ref, &status)
            .await?;
    }

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
