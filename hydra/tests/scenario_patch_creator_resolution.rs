mod harness;

use anyhow::Result;
use harness::{test_job_settings, TestHarness};
use hydra_common::{
    issues::{IssueStatus, IssueType},
    task_status::Status,
    users::Username,
};
use std::str::FromStr;

/// Scenario 15a: User creates patch directly → `patch.creator` resolves to the
/// user's username.
#[tokio::test]
async fn patch_creator_resolves_to_user_for_direct_patch() -> Result<()> {
    let harness = TestHarness::builder()
        .with_repo("acme/creator-test")
        .with_user("alice")
        .build()
        .await?;

    let alice = harness.user("alice");
    let repo = hydra_common::RepoName::from_str("acme/creator-test")?;

    let patch_id = alice
        .create_patch("Direct user patch", "Created by alice directly", &repo)
        .await?;

    let patch = alice.get_patch(&patch_id).await?;
    assert_eq!(
        patch.patch.creator,
        Username::from("alice"),
        "patch.creator should resolve to alice for a direct user patch"
    );

    Ok(())
}

/// Scenario 15b: Agent creates patch → `patch.creator` resolved from the
/// originating issue's creator (the human), not the agent that executed the
/// worker.
#[tokio::test]
async fn patch_creator_resolves_to_issue_creator_for_agent_patch() -> Result<()> {
    let harness = TestHarness::builder()
        .with_repo("acme/agent-creator-test")
        .with_agent("swe", "implement features")
        .build()
        .await?;

    let user = harness.default_user();
    let repo = hydra_common::RepoName::from_str("acme/agent-creator-test")?;

    let issue_id = user
        .create_issue_with_settings(
            "Fix authentication bug",
            IssueType::Task,
            IssueStatus::Open,
            Some("swe"),
            Some(test_job_settings(&repo)),
        )
        .await?;

    let task_ids = harness.await_sessions(&issue_id, 1).await?;
    assert_eq!(task_ids.len(), 1);
    let job_id = &task_ids[0];

    let result = harness
        .run_worker(
            job_id,
            vec![
                "echo 'fix auth' >> auth.rs",
                "git add auth.rs",
                "git commit -m 'Fix authentication bug'",
                "hydra patches create --title 'Fix auth bug' --description 'Fixes the authentication issue'",
            ],
        )
        .await?;

    assert_eq!(result.final_status, Status::Complete);
    assert_eq!(result.patches_created.len(), 1);

    let patch = user.get_patch(&result.patches_created[0]).await?;
    assert_eq!(
        patch.patch.creator,
        Username::from("default"),
        "patch.creator should resolve to the issue creator (default user), not the agent"
    );

    let _ = job_id;

    Ok(())
}
