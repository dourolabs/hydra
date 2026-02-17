mod harness;

use anyhow::Result;
use harness::{test_job_settings_full, test_patch_workflow_config, IssueAssertions};
use metis_common::{
    issues::{IssueStatus, IssueType},
    patches::{PatchStatus, SearchPatchesQuery, UpsertPatchRequest},
    RepoName,
};
use std::str::FromStr;

// ── Scenario 12: Automatic Backup Patches Are Excluded from Workflow ────

/// Backup patches (is_automatic_backup: true) must NOT trigger patch_workflow.
/// Normal patches DO trigger it.
///
/// The worker_run pipeline automatically creates a backup patch at the end
/// of a worker run. This backup patch should not trigger the patch_workflow
/// automation. We verify this by running a worker that creates a normal
/// patch, then checking that only the normal patch has workflow issues.
#[tokio::test]
async fn backup_patches_do_not_trigger_patch_workflow() -> Result<()> {
    let repo = RepoName::from_str("acme/app")?;

    let harness = harness::TestHarness::builder()
        .with_repo("acme/app")
        .with_agent("swe", "Implement changes")
        .with_patch_workflow_config(test_patch_workflow_config("reviewer", None))
        .build()
        .await?;
    let user = harness.default_user();

    // Create an issue and spawn a task.
    let _swe_issue_id = user
        .create_issue_with_settings(
            "Backup patch test",
            IssueType::Task,
            IssueStatus::Open,
            Some("swe"),
            Some(test_job_settings_full(&repo, "worker:latest", "main")),
        )
        .await?;

    let task_ids = harness.step_schedule().await?;
    assert_eq!(task_ids.len(), 1);

    // Run the worker, which creates a normal patch via explicit CLI command.
    // The worker_run pipeline may also create an automatic backup patch at
    // the end of the run.
    let result = harness
        .run_worker(
            &task_ids[0],
            vec![
                "echo 'real fix' >> README.md",
                "git add README.md",
                "git commit -m 'real fix'",
                "metis patches create --title 'Real fix' --description 'Normal patch'",
            ],
        )
        .await?;
    assert_eq!(result.patches_created.len(), 1);
    let normal_patch_id = result.patches_created[0].clone();

    // Query all patches from the server (including backups).
    let client = harness.client()?;
    let all_patches = client.list_patches(&SearchPatchesQuery::default()).await?;

    // Identify backup patches (if any were created by worker_run).
    let backup_patches: Vec<_> = all_patches
        .patches
        .iter()
        .filter(|p| p.patch.is_automatic_backup)
        .collect();

    // Verify that any backup patches did NOT trigger workflow issues.
    let all_issues = user.list_issues().await?;
    let workflow_issues: Vec<_> = all_issues
        .issues
        .iter()
        .filter(|i| {
            i.issue.issue_type == IssueType::ReviewRequest
                || i.issue.issue_type == IssueType::MergeRequest
        })
        .collect();

    // Workflow issues should only reference the normal patch, never a backup.
    for wi in &workflow_issues {
        assert!(
            wi.issue.patches.contains(&normal_patch_id),
            "workflow issue should reference the normal patch"
        );
        for backup in &backup_patches {
            assert!(
                !wi.issue.patches.contains(&backup.patch_id),
                "workflow issue should not reference backup patch {:?}",
                backup.patch_id
            );
        }
    }

    // Verify that the normal patch DID trigger workflow (ReviewRequest + MergeRequest).
    assert!(
        workflow_issues.len() >= 2,
        "normal patch should trigger workflow, found {} workflow issues",
        workflow_issues.len()
    );

    Ok(())
}

// ── Scenario 13: Patch Closure Drops Review Workflow Issues ────

/// When a patch is closed without merging, its ReviewRequest issues should
/// be dropped and its MergeRequest issues should be failed.
#[tokio::test]
async fn closing_patch_drops_review_workflow_issues() -> Result<()> {
    let repo = RepoName::from_str("acme/app")?;

    let harness = harness::TestHarness::builder()
        .with_repo("acme/app")
        .with_agent("swe", "Implement changes")
        .with_patch_workflow_config(test_patch_workflow_config("reviewer", None))
        .build()
        .await?;
    let user = harness.default_user();

    // Create an issue and spawn a task.
    let _swe_issue_id = user
        .create_issue_with_settings(
            "Patch closure test",
            IssueType::Task,
            IssueStatus::Open,
            Some("swe"),
            Some(test_job_settings_full(&repo, "worker:latest", "main")),
        )
        .await?;

    let task_ids = harness.step_schedule().await?;
    assert_eq!(task_ids.len(), 1);

    // SWE worker creates a patch.
    let result = harness
        .run_worker(
            &task_ids[0],
            vec![
                "echo 'changes' >> README.md",
                "git add README.md",
                "git commit -m 'changes'",
                "metis patches create --title 'Abandoned patch' --description 'Will be closed'",
            ],
        )
        .await?;
    assert_eq!(result.patches_created.len(), 1);
    let patch_id = result.patches_created[0].clone();

    // Verify workflow issues exist.
    let all_issues = user.list_issues().await?;
    let review_request = all_issues
        .issues
        .iter()
        .find(|i| i.issue.issue_type == IssueType::ReviewRequest)
        .expect("expected ReviewRequest issue");
    let merge_request = all_issues
        .issues
        .iter()
        .find(|i| i.issue.issue_type == IssueType::MergeRequest)
        .expect("expected MergeRequest issue");

    review_request.assert_status(IssueStatus::Open);
    merge_request.assert_status(IssueStatus::Open);

    // Close the patch (abandon it).
    let client = harness.client()?;
    let mut patch_record = client.get_patch(&patch_id).await?;
    patch_record.patch.status = PatchStatus::Closed;
    client
        .update_patch(&patch_id, &UpsertPatchRequest::new(patch_record.patch))
        .await?;

    // Verify the close_merge_request_issues automation ran.
    let review_request_updated = user.get_issue(&review_request.issue_id).await?;
    let merge_request_updated = user.get_issue(&merge_request.issue_id).await?;

    // MergeRequest should be Failed.
    merge_request_updated.assert_status(IssueStatus::Failed);

    // ReviewRequest should be Dropped.
    review_request_updated.assert_status(IssueStatus::Dropped);

    Ok(())
}
