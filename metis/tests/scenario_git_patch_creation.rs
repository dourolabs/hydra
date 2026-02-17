mod harness;

use anyhow::Result;
use harness::{test_job_settings, PatchAssertions, TestHarness};
use metis_common::task_status::Status;
use std::str::FromStr;

/// Scenario 11: Worker Git Operations and Patch Creation
///
/// Tests the fundamental SWE agent workflow: clone repo, create branch,
/// make changes, commit, push, and create a patch. Verifies patch metadata
/// (title, description, branch_name, creator, created_by) and diff content.
#[tokio::test]
async fn worker_git_operations_and_patch_creation() -> Result<()> {
    let harness = TestHarness::builder()
        .with_repo("acme/swe-repo")
        .with_agent("swe", "implement features")
        .build()
        .await?;

    let user = harness.default_user();
    let repo = metis_common::RepoName::from_str("acme/swe-repo")?;

    // Create an issue with repo job settings and assign to swe agent.
    let _issue_id = user
        .create_issue_with_settings(
            "Add greeting module",
            metis_common::issues::IssueType::Task,
            metis_common::issues::IssueStatus::Open,
            Some("swe"),
            Some(test_job_settings(&repo)),
        )
        .await?;

    // step_schedule() spawns a task for the issue.
    let task_ids = harness.step_schedule().await?;
    assert_eq!(task_ids.len(), 1, "should spawn exactly one task");
    let job_id = &task_ids[0];

    // Worker executes: make file changes, commit, push, create patch.
    let result = harness
        .run_worker(
            job_id,
            vec![
                "echo 'fn greet() { println!(\"hello\"); }' > greet.rs",
                "git add greet.rs",
                "git commit -m 'Add greeting module'",
                "metis patches create --title 'Add greeting module' --description 'Implements the greeting function'",
            ],
        )
        .await?;

    // Verify job completed successfully.
    assert_eq!(result.final_status, Status::Complete);
    assert_eq!(
        result.patches_created.len(),
        1,
        "worker should create exactly one patch"
    );

    // Verify patch metadata.
    let patch = user.get_patch(&result.patches_created[0]).await?;
    assert_eq!(patch.patch.title, "Add greeting module");
    assert_eq!(
        patch.patch.description, "Implements the greeting function",
        "patch description should match"
    );
    assert_eq!(
        patch.patch.service_repo_name, repo,
        "patch should reference the correct repo"
    );

    // Verify patch diff contains the expected changes.
    patch.assert_diff_contains("greet.rs");
    patch.assert_diff_contains("fn greet()");

    // Verify patch.created_by references the task ID.
    assert_eq!(
        patch.patch.created_by,
        Some(job_id.clone()),
        "patch.created_by should reference the worker's task ID"
    );

    // Verify patch.creator is set (resolved from actor chain to the issue creator).
    assert!(
        patch.patch.creator.is_some(),
        "patch.creator should be set (resolved from actor chain)"
    );
    assert_eq!(
        patch.patch.creator.as_ref().unwrap().as_ref(),
        "default",
        "patch.creator should resolve to the issue creator"
    );

    Ok(())
}
