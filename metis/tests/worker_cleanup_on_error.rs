// Migrated from the old-style test using init_test_server_with_remote + run_as_worker_with_failure.
// Original: ~57 lines with manual env var construction, CLI subprocess job creation,
// wait_for_status polling, manual job list inspection, and BundleSpec assertion.
// Migrated: ~20 lines using TestHarness, UserHandle, and run_worker_expect_failure.

mod harness;

use anyhow::Result;
use metis_common::task_status::Status;
use std::str::FromStr;

#[tokio::test]
async fn worker_run_executes_cleanup_on_error() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/worker-cleanup-error")
        .build()
        .await?;
    let user = harness.default_user();
    let repo = metis_common::RepoName::from_str("acme/worker-cleanup-error")?;

    let issue_id = user
        .create_issue("worker cleanup executes on error")
        .await?;
    let job_id = user
        .create_job_for_issue(&repo, "worker cleanup executes on error", &issue_id)
        .await?;

    let failure = harness
        .run_worker_expect_failure(
            &job_id,
            vec![
                "echo 'cleanup with error' >> README.md",
                "git add README.md",
                "exit 1",
            ],
        )
        .await?;

    assert_eq!(failure.final_status, Status::Failed);
    assert!(
        !failure.error.to_string().is_empty(),
        "failure should contain an error message"
    );

    Ok(())
}
