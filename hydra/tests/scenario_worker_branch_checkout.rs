mod harness;

use anyhow::Result;
use harness::{test_job_settings, TestHarness};
use hydra_common::task_status::Status;
use std::str::FromStr;

/// Scenario: Worker switches to a non-default branch using `git checkout`.
///
/// 1. Creates a git remote with a feature branch containing a unique file.
/// 2. Registers the repo and creates an issue targeting main.
/// 3. Spawns a task and runs a worker on main.
/// 4. The worker uses `git checkout <branch>` to switch branches.
/// 5. Verifies the feature-branch file is present in the working tree.
/// 6. Makes a commit and creates a patch from the feature branch.
/// 7. Validates job completion and patch contents.
#[tokio::test]
async fn worker_checks_out_non_default_branch() -> Result<()> {
    let harness = TestHarness::builder()
        .with_repo("acme/branch-test")
        .with_agent("swe", "implement features")
        .build()
        .await?;

    let user = harness.default_user();
    let repo = hydra_common::RepoName::from_str("acme/branch-test")?;

    // Create a feature branch on the remote with a unique file.
    let remote = harness.remote("acme/branch-test");
    remote.create_branch("feature-xyz", "feature.txt", "feature content\n")?;

    // Create an issue (targeting main by default) and schedule a task.
    let _issue_id = user
        .create_issue_with_settings(
            "Test branch checkout",
            hydra_common::issues::IssueType::Task,
            hydra_common::issues::IssueStatus::Open,
            Some("swe"),
            Some(test_job_settings(&repo)),
        )
        .await?;

    let task_ids = harness.step_schedule().await?;
    assert_eq!(task_ids.len(), 1, "should spawn exactly one task");
    let job_id = &task_ids[0];

    // Worker starts on main, switches to feature-xyz using git checkout
    // (all remote branches are already fetched during worker init),
    // verifies the file, makes a change, commits, and creates a patch.
    let result = harness
        .run_worker(
            job_id,
            vec![
                "git checkout feature-xyz",
                "test -f feature.txt",
                "cat feature.txt",
                "echo 'additional work' >> feature.txt",
                "git add feature.txt",
                "git commit -m 'Extend feature on feature-xyz'",
                "metis patches create --title 'Feature branch work' --description 'Changes made after checking out feature-xyz'",
            ],
        )
        .await?;

    assert_eq!(
        result.final_status,
        Status::Complete,
        "job should complete successfully"
    );

    // Verify the checkout actually read the feature branch file.
    let cat_output = &result.outputs[2];
    assert!(
        cat_output.stdout.contains("feature content"),
        "feature.txt should contain the content from the feature branch"
    );

    // Verify a patch was created.
    assert_eq!(
        result.patches_created.len(),
        1,
        "worker should create exactly one patch"
    );
    let patch = user.get_patch(&result.patches_created[0]).await?;
    assert_eq!(patch.patch.title, "Feature branch work");

    Ok(())
}
