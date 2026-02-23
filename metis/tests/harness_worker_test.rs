mod harness;

use anyhow::Result;
use metis_common::task_status::Status;
use std::str::FromStr;

/// Integration test: create issue -> create job -> run_worker with git commit
/// + patch create -> verify patch exists and job completes.
#[tokio::test]
async fn run_worker_creates_patch() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/worker-test")
        .build()
        .await?;
    let user = harness.default_user();

    let repo = metis_common::RepoName::from_str("acme/worker-test")?;
    let issue_id = user.create_issue("worker patch integration test").await?;
    let job_id = user
        .create_job_for_issue(&repo, "worker patch integration test", &issue_id)
        .await?;

    let result = harness
        .run_worker(
            &job_id,
            vec![
                "echo 'worker content' >> README.md",
                "git add README.md",
                "git commit -m 'worker changes'",
                "metis patches create --title 'harness worker patch' --description 'created by harness worker'",
            ],
        )
        .await?;

    assert_eq!(
        result.final_status,
        Status::Complete,
        "job should complete after successful worker run"
    );
    assert_eq!(
        result.patches_created.len(),
        1,
        "worker should create exactly one non-backup patch"
    );

    // Verify the patch content through the API.
    let patch = user.get_patch(&result.patches_created[0]).await?;
    assert_eq!(patch.patch.title, "harness worker patch");

    Ok(())
}

/// Verify that run_worker returns captured command outputs.
#[tokio::test]
async fn run_worker_captures_command_outputs() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/outputs-test")
        .build()
        .await?;
    let user = harness.default_user();

    let repo = metis_common::RepoName::from_str("acme/outputs-test")?;
    let job_id = user.create_job(&repo, "output capture test").await?;

    let result = harness
        .run_worker(&job_id, vec!["echo hello world"])
        .await?;

    assert!(!result.outputs.is_empty(), "should have captured outputs");
    assert!(
        result.outputs[0].stdout.contains("hello world"),
        "captured stdout should contain echo output"
    );
    assert_eq!(result.outputs[0].status, 0, "echo should succeed");

    Ok(())
}

/// Verify that --tempdir causes worker commands to run inside a temporary
/// directory rather than the original destination path.
#[tokio::test]
async fn run_worker_with_tempdir_uses_temp_dir_as_cwd() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/tempdir-test")
        .build()
        .await?;
    let user = harness.default_user();

    let repo = metis_common::RepoName::from_str("acme/tempdir-test")?;
    let job_id = user.create_job(&repo, "tempdir cwd test").await?;

    let result = harness
        .run_worker_with_tempdir(&job_id, vec!["pwd"])
        .await?;

    assert_eq!(
        result.final_status,
        Status::Complete,
        "job should complete after successful worker run with --tempdir"
    );
    assert!(!result.outputs.is_empty(), "should have captured outputs");

    // The working directory should be a /repo subdirectory inside a temp path,
    // not the original (non-empty) destination directory.
    let cwd = result.outputs[0].stdout.trim();
    assert!(
        cwd.contains("/repo"),
        "worker cwd should be a repo subdirectory, got: {cwd}"
    );
    let temp_dir_prefix = std::env::temp_dir();
    assert!(
        cwd.starts_with(temp_dir_prefix.to_str().unwrap()),
        "worker cwd should be inside the system temp directory, got: {cwd}"
    );

    Ok(())
}

/// Verify that run_worker_expect_failure returns WorkerFailure when a command fails.
#[tokio::test]
async fn run_worker_expect_failure_captures_error() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/fail-test")
        .build()
        .await?;
    let user = harness.default_user();

    let repo = metis_common::RepoName::from_str("acme/fail-test")?;
    let job_id = user.create_job(&repo, "failure test").await?;

    let failure = harness
        .run_worker_expect_failure(&job_id, vec!["exit 1"])
        .await?;

    assert_eq!(
        failure.final_status,
        Status::Failed,
        "job should be marked as failed after worker failure"
    );
    assert!(
        !failure.error.to_string().is_empty(),
        "failure should contain an error message"
    );

    Ok(())
}
