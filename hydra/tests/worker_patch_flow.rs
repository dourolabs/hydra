mod harness;

use anyhow::Result;
use hydra_common::task_status::Status;
use std::str::FromStr;

#[tokio::test]
async fn worker_run_creates_patch_via_override_command() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/worker-test")
        .build()
        .await?;
    let user = harness.default_user();
    let repo = hydra_common::RepoName::from_str("acme/worker-test")?;

    let issue_id = user.create_issue("worker integration patch flow").await?;
    let job_id = user
        .create_session_for_issue(&repo, "worker integration patch flow", &issue_id)
        .await?;

    let result = harness
        .run_worker(
            &job_id,
            vec![
                "echo 'worker content' >> README.md",
                "git add README.md",
                "git commit -m 'worker changes'",
                "hydra patches create --title 'integration worker patch' --description 'created by worker override'",
            ],
        )
        .await?;

    assert_eq!(result.final_status, Status::Complete);
    assert_eq!(result.patches_created.len(), 1);

    let patch = user.get_patch(&result.patches_created[0]).await?;
    assert_eq!(patch.patch.title, "integration worker patch");
    assert_eq!(
        patch.patch.service_repo_name,
        hydra_common::RepoName::from_str("acme/worker-test")?
    );

    Ok(())
}
