// Migrated from the old-style test using init_test_server_with_remote + run_as_user + run_as_worker.
// Original: ~70 lines with manual env var construction, CLI subprocess job creation,
// job_id_for_prompt polling, and manual patch/job list inspection.
// Migrated: ~25 lines using TestHarness, UserHandle, and run_worker.

mod harness;

use anyhow::Result;
use metis_common::task_status::Status;
use std::str::FromStr;

#[tokio::test]
async fn worker_run_creates_patch_via_override_command() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/worker-test")
        .build()
        .await?;
    let user = harness.default_user();
    let repo = metis_common::RepoName::from_str("acme/worker-test")?;

    let issue_id = user.create_issue("worker integration patch flow").await?;
    let job_id = user
        .create_job_for_issue(&repo, "worker integration patch flow", &issue_id)
        .await?;

    let result = harness
        .run_worker(
            &job_id,
            vec![
                "echo 'worker content' >> README.md",
                "git add README.md",
                "git commit -m 'worker changes'",
                "metis patches create --title 'integration worker patch' --description 'created by worker override'",
            ],
        )
        .await?;

    assert_eq!(result.final_status, Status::Complete);
    assert_eq!(result.patches_created.len(), 1);

    let patch = user.get_patch(&result.patches_created[0]).await?;
    assert_eq!(patch.patch.title, "integration worker patch");
    assert_eq!(
        patch.patch.service_repo_name,
        metis_common::RepoName::from_str("acme/worker-test")?
    );

    Ok(())
}
