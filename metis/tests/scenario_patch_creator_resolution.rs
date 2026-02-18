mod harness;

use anyhow::Result;
use harness::{
    test_job_settings, MergeRequestConfig, PatchWorkflowConfig, ReviewRequestConfig, TestHarness,
};
use metis_common::{
    issues::{IssueDependencyType, IssueStatus, IssueType},
    task_status::Status,
    users::Username,
};
use std::str::FromStr;

/// Scenario 15a: User creates patch directly → $patch_creator resolves to user's username.
///
/// Also verifies that the patch_workflow automation fires and creates
/// ReviewRequest/MergeRequest issues assigned via $patch_creator substitution.
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

    // Flush automations so patch_workflow creates ReviewRequest + MergeRequest.
    harness.flush_automations().await?;

    // Verify patch.creator resolves to alice's username.
    let patch = alice.get_patch(&patch_id).await?;
    assert_eq!(
        patch.patch.creator,
        Username::from("alice"),
        "$patch_creator should resolve to alice for a direct user patch"
    );

    // Verify patch_workflow automation created ReviewRequest and MergeRequest
    // issues with $patch_creator resolved to "alice".
    let all_issues = alice.list_issues().await?;

    let review_requests: Vec<_> = all_issues
        .issues
        .iter()
        .filter(|i| i.issue.issue_type == IssueType::ReviewRequest)
        .collect();
    assert_eq!(
        review_requests.len(),
        1,
        "patch_workflow should create 1 ReviewRequest issue"
    );
    assert_eq!(
        review_requests[0].issue.assignee,
        Some("alice".to_string()),
        "ReviewRequest should be assigned to alice via $patch_creator"
    );

    let merge_requests: Vec<_> = all_issues
        .issues
        .iter()
        .filter(|i| i.issue.issue_type == IssueType::MergeRequest)
        .collect();
    assert_eq!(
        merge_requests.len(),
        1,
        "patch_workflow should create 1 MergeRequest issue"
    );
    assert_eq!(
        merge_requests[0].issue.assignee,
        Some("alice".to_string()),
        "MergeRequest should be assigned to alice via $patch_creator"
    );

    Ok(())
}

/// Scenario 15b: Agent creates patch → patch.creator resolved from Actor.creator
/// (the human who created the originating issue).
///
/// Also verifies that the patch_workflow automation fires and creates
/// ReviewRequest/MergeRequest child issues of the SWE's issue, assigned to
/// the original issue creator via $patch_creator substitution.
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
    let issue_id = user
        .create_issue_with_settings(
            "Fix authentication bug",
            IssueType::Task,
            IssueStatus::Open,
            Some("swe"),
            Some(test_job_settings(&repo)),
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

    // Flush automations so patch_workflow creates ReviewRequest + MergeRequest.
    harness.flush_automations().await?;

    // Verify patch.creator is set to the original user who created the issue,
    // not the agent/task that executed the worker.
    let patch = user.get_patch(&result.patches_created[0]).await?;
    assert_eq!(
        patch.patch.creator,
        Username::from("default"),
        "patch.creator should resolve to the issue creator (default user), not the agent"
    );

    // Verify created_by references the task ID.
    assert_eq!(
        patch.patch.created_by,
        Some(job_id.clone()),
        "patch.created_by should reference the worker's task ID"
    );

    // Verify patch_workflow automation created ReviewRequest and MergeRequest
    // child issues of the SWE's issue, with $patch_creator resolved to "default".
    let all_issues = user.list_issues().await?;

    let review_requests: Vec<_> = all_issues
        .issues
        .iter()
        .filter(|i| {
            i.issue.issue_type == IssueType::ReviewRequest
                && i.issue.dependencies.iter().any(|d| {
                    d.dependency_type == IssueDependencyType::ChildOf && d.issue_id == issue_id
                })
        })
        .collect();
    assert_eq!(
        review_requests.len(),
        1,
        "patch_workflow should create 1 ReviewRequest child issue"
    );
    assert_eq!(
        review_requests[0].issue.assignee,
        Some("default".to_string()),
        "ReviewRequest should be assigned to 'default' (issue creator) via $patch_creator"
    );

    let merge_requests: Vec<_> = all_issues
        .issues
        .iter()
        .filter(|i| {
            i.issue.issue_type == IssueType::MergeRequest
                && i.issue.dependencies.iter().any(|d| {
                    d.dependency_type == IssueDependencyType::ChildOf && d.issue_id == issue_id
                })
        })
        .collect();
    assert_eq!(
        merge_requests.len(),
        1,
        "patch_workflow should create 1 MergeRequest child issue"
    );
    assert_eq!(
        merge_requests[0].issue.assignee,
        Some("default".to_string()),
        "MergeRequest should be assigned to 'default' (issue creator) via $patch_creator"
    );

    // Verify MergeRequest is blocked-on ReviewRequest.
    let rr_id = &review_requests[0].issue_id;
    assert!(
        merge_requests[0].issue.dependencies.iter().any(|d| {
            d.dependency_type == IssueDependencyType::BlockedOn && d.issue_id == *rr_id
        }),
        "MergeRequest should be blocked-on ReviewRequest"
    );

    Ok(())
}
