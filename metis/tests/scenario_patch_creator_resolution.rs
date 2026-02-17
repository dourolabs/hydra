mod harness;

use anyhow::Result;
use harness::TestHarness;
use metis_common::{
    issues::{IssueStatus, IssueType, JobSettings},
    task_status::Status,
    users::Username,
};
use metis_server::policy::automations::patch_workflow::{
    MergeRequestConfig, PatchWorkflowConfig, ReviewRequestConfig,
};
use std::str::FromStr;

/// Scenario 15a: User creates patch directly → $patch_creator resolves to user's username.
#[tokio::test]
async fn patch_creator_resolves_to_user_for_direct_patch() -> Result<()> {
    let harness = TestHarness::builder()
        .with_repo("acme/creator-test")
        .with_user("alice")
        .with_patch_workflow_config(PatchWorkflowConfig {
            review_requests: vec![ReviewRequestConfig {
                assignee: "$patch_creator".to_string(),
            }],
            merge_request: Some(MergeRequestConfig {
                assignee: Some("$patch_creator".to_string()),
            }),
            repos: Default::default(),
        })
        .build()
        .await?;

    let alice = harness.user("alice");
    let repo = metis_common::RepoName::from_str("acme/creator-test")?;

    // Alice creates a patch directly.
    let patch_id = alice
        .create_patch("Direct user patch", "Created by alice directly", &repo)
        .await?;

    // Verify patch.creator resolves to alice's username.
    let patch = alice.get_patch(&patch_id).await?;
    assert_eq!(
        patch.patch.creator,
        Some(Username::from("alice")),
        "$patch_creator should resolve to alice for a direct user patch"
    );

    Ok(())
}

/// Scenario 15b: Agent creates patch → patch.creator resolved from Actor.creator
/// (the human who created the originating issue).
#[tokio::test]
async fn patch_creator_resolves_to_issue_creator_for_agent_patch() -> Result<()> {
    let harness = TestHarness::builder()
        .with_repo("acme/agent-creator-test")
        .with_agent("swe", "implement features")
        .with_patch_workflow_config(PatchWorkflowConfig {
            review_requests: vec![ReviewRequestConfig {
                assignee: "$patch_creator".to_string(),
            }],
            merge_request: Some(MergeRequestConfig {
                assignee: Some("$patch_creator".to_string()),
            }),
            repos: Default::default(),
        })
        .build()
        .await?;

    let user = harness.default_user();
    let repo = metis_common::RepoName::from_str("acme/agent-creator-test")?;

    // User creates an issue assigned to SWE agent.
    let mut job_settings = JobSettings::default();
    job_settings.repo_name = Some(repo.clone());

    let _issue_id = user
        .create_issue_with_settings(
            "Fix authentication bug",
            IssueType::Task,
            IssueStatus::Open,
            Some("swe"),
            Some(job_settings),
        )
        .await?;

    // step_schedule() spawns a task for the issue.
    let task_ids = harness.step_schedule().await?;
    assert_eq!(task_ids.len(), 1);
    let job_id = &task_ids[0];

    // SWE worker creates a patch.
    let result = harness
        .run_worker(
            job_id,
            vec![
                "echo 'fix auth' >> auth.rs",
                "git add auth.rs",
                "git commit -m 'Fix authentication bug'",
                "metis patches create --title 'Fix auth bug' --description 'Fixes the authentication issue'",
            ],
        )
        .await?;

    assert_eq!(result.final_status, Status::Complete);
    assert_eq!(result.patches_created.len(), 1);

    // Verify patch.creator is set to the original user who created the issue,
    // not the agent/task that executed the worker.
    let patch = user.get_patch(&result.patches_created[0]).await?;
    assert!(
        patch.patch.creator.is_some(),
        "patch.creator should be set for agent-created patches"
    );
    assert_eq!(
        patch.patch.creator,
        Some(Username::from("default")),
        "patch.creator should resolve to the issue creator (default user), not the agent"
    );

    // Verify created_by references the task ID.
    assert_eq!(
        patch.patch.created_by,
        Some(job_id.clone()),
        "patch.created_by should reference the worker's task ID"
    );

    Ok(())
}
