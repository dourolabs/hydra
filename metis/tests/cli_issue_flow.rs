// Migrated from old-style using test_state_handles + spawn_test_server_with_state + manual
// actor/issue/server setup + run_metis_command helper.
// Migrated to TestHarness and UserHandle::cli.

mod harness;

use anyhow::{anyhow, Result};
use metis_common::issues::{IssueStatus, IssueType, JobSettings};
use std::str::FromStr;

#[tokio::test]
async fn cli_issue_flow_creates_and_lists_issue() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/cli-flow")
        .build()
        .await?;
    let user = harness.default_user();

    // Create a parent issue with specific job settings that should be inherited.
    let mut parent_job_settings = JobSettings::default();
    parent_job_settings.repo_name =
        Some(metis_common::RepoName::from_str("acme/cli-flow").unwrap());
    parent_job_settings.remote_url = Some("https://example.com/cli-flow.git".into());
    parent_job_settings.image = Some("worker:latest".into());
    parent_job_settings.branch = Some("feature/cli-flow".into());

    let parent_id = user
        .create_issue_with_settings(
            "parent issue",
            IssueType::Task,
            IssueStatus::Open,
            None,
            Some(parent_job_settings),
        )
        .await?;

    let description = "integration flow issue";
    let deps_arg = format!("child-of:{parent_id}");

    // Create a child issue via CLI, passing the parent as --current-issue-id for inheritance.
    user.cli(&[
        "issues",
        "create",
        "--deps",
        &deps_arg,
        "--current-issue-id",
        parent_id.as_ref(),
        description,
    ])
    .await?;

    // List issues via CLI to verify listing works.
    user.cli(&["issues", "list"]).await?;

    // Verify the created issue inherited job settings from the parent.
    let issues = user.list_issues().await?.issues;
    let created = issues
        .iter()
        .find(|issue| issue.issue.description == description)
        .ok_or_else(|| anyhow!("expected issue to be created"))?;

    assert_eq!(created.issue.status, IssueStatus::Open);
    assert_eq!(
        created.issue.job_settings.repo_name,
        Some(metis_common::RepoName::from_str("acme/cli-flow").unwrap())
    );
    assert_eq!(
        created.issue.job_settings.remote_url,
        Some("https://example.com/cli-flow.git".into())
    );
    assert_eq!(
        created.issue.job_settings.image,
        Some("worker:latest".into())
    );
    assert_eq!(
        created.issue.job_settings.branch,
        Some("feature/cli-flow".into())
    );

    Ok(())
}
