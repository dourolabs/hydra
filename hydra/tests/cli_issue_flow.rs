mod harness;

use anyhow::{anyhow, Result};
use hydra_common::issues::{IssueStatus, IssueType, SessionSettings};
use std::str::FromStr;

#[tokio::test]
async fn cli_issue_flow_creates_and_lists_issue() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/cli-flow")
        .build()
        .await?;
    let user = harness.default_user();

    // Create a parent issue with specific job settings that should be inherited.
    let mut parent_session_settings = SessionSettings::default();
    parent_session_settings.repo_name =
        Some(hydra_common::RepoName::from_str("acme/cli-flow").unwrap());
    parent_session_settings.remote_url = Some("https://example.com/cli-flow.git".into());
    parent_session_settings.image = Some("worker:latest".into());
    parent_session_settings.branch = Some("feature/cli-flow".into());

    let parent_id = user
        .create_issue_with_settings(
            "parent issue",
            IssueType::Task,
            IssueStatus::Open,
            None,
            Some(parent_session_settings),
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
    // list_issues() returns IssueSummaryRecord which excludes session_settings,
    // so find the issue ID from the summary and then fetch the full record.
    let issues = user.list_issues().await?.issues;
    let created_summary = issues
        .iter()
        .find(|issue| issue.issue.description == description)
        .ok_or_else(|| anyhow!("expected issue to be created"))?;

    assert_eq!(created_summary.issue.status, IssueStatus::Open);

    let created = user.get_issue(&created_summary.issue_id).await?;
    assert_eq!(
        created.issue.session_settings.repo_name,
        Some(hydra_common::RepoName::from_str("acme/cli-flow").unwrap())
    );
    assert_eq!(
        created.issue.session_settings.remote_url,
        Some("https://example.com/cli-flow.git".into())
    );
    assert_eq!(
        created.issue.session_settings.image,
        Some("worker:latest".into())
    );
    assert_eq!(
        created.issue.session_settings.branch,
        Some("feature/cli-flow".into())
    );

    Ok(())
}
