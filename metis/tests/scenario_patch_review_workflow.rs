mod harness;

use anyhow::{Context, Result};
use harness::IssueAssertions;
use metis_common::{
    issues::{IssueStatus, IssueType, JobSettings},
    patches::{Patch, PatchStatus, UpsertPatchRequest},
    RepoName,
};
use metis_server::policy::automations::patch_workflow::{
    MergeRequestConfig, PatchWorkflowConfig, ReviewRequestConfig,
};
use std::str::FromStr;

// ── Scenario 2: Patch Workflow — Review and Merge Request Automation ────

/// Full patch_workflow automation test:
///
/// SWE creates a patch → automation creates ReviewRequest + MergeRequest
/// as children of SWE's issue → reviewer approves → patch merged →
/// MergeRequest closed → SWE's issue can close.
#[tokio::test]
async fn patch_workflow_creates_review_and_merge_request_issues() -> Result<()> {
    let repo = RepoName::from_str("acme/app")?;
    let pwc = PatchWorkflowConfig {
        review_requests: vec![ReviewRequestConfig {
            assignee: "reviewer".to_string(),
        }],
        merge_request: Some(MergeRequestConfig {
            assignee: Some("merger".to_string()),
        }),
        repos: Default::default(),
    };

    let harness = harness::TestHarness::builder()
        .with_repo("acme/app")
        .with_user("reviewer")
        .with_agent("swe", "Implement changes")
        .with_patch_workflow_config(pwc)
        .build()
        .await?;
    let user = harness.default_user();

    // 1. Create an issue (SWE's issue) with job settings.
    let mut job_settings = JobSettings::default();
    job_settings.repo_name = Some(repo.clone());
    job_settings.image = Some("worker:latest".to_string());
    job_settings.branch = Some("main".to_string());

    let swe_issue_id = user
        .create_issue_with_settings(
            "Fix login bug",
            IssueType::Task,
            IssueStatus::Open,
            Some("swe"),
            Some(job_settings),
        )
        .await?;

    // 2. step_schedule() spawns a task for the SWE's issue.
    let task_ids = harness.step_schedule().await?;
    assert_eq!(task_ids.len(), 1, "expected one task to be spawned");

    // 3. SWE worker creates a patch with a branch name.
    let result = harness
        .run_worker(
            &task_ids[0],
            vec![
                "echo 'fix login' >> README.md",
                "git add README.md",
                "git commit -m 'fix login bug'",
                "metis patches create --title 'Fix login bug' --description 'Fixes the login flow'",
            ],
        )
        .await?;
    assert_eq!(result.patches_created.len(), 1);
    let patch_id = result.patches_created[0].clone();

    // 4. Verify patch_workflow automation fired and created children.
    let all_issues = user.list_issues().await?;
    let swe_issue = user.get_issue(&swe_issue_id).await?;

    // ReviewRequest should be a child of SWE's issue, assigned to "reviewer".
    swe_issue.assert_has_child_with_status(
        &all_issues.issues,
        "Review request for patch",
        IssueStatus::Open,
    );

    // MergeRequest should be a child of SWE's issue.
    swe_issue.assert_has_child_with_status(&all_issues.issues, "Review patch", IssueStatus::Open);

    // Find the ReviewRequest and MergeRequest issue IDs.
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

    assert_eq!(review_request.issue.assignee.as_deref(), Some("reviewer"));
    assert_eq!(merge_request.issue.assignee.as_deref(), Some("merger"));

    // Verify MergeRequest is blocked on ReviewRequest.
    let is_blocked = merge_request.issue.dependencies.iter().any(|dep| {
        dep.dependency_type == metis_common::issues::IssueDependencyType::BlockedOn
            && dep.issue_id == review_request.issue_id
    });
    assert!(
        is_blocked,
        "MergeRequest should be blocked on ReviewRequest"
    );

    // Verify patches are attached to the workflow issues.
    assert!(
        review_request.issue.patches.contains(&patch_id),
        "ReviewRequest should reference the patch"
    );
    assert!(
        merge_request.issue.patches.contains(&patch_id),
        "MergeRequest should reference the patch"
    );

    // 5. Verify SWE's issue cannot be closed (has open children).
    let close_result = user
        .update_issue_status(&swe_issue_id, IssueStatus::Closed)
        .await;
    assert!(
        close_result.is_err(),
        "SWE's issue should not be closable while children are open"
    );

    // 6. Reviewer approves the patch via CLI.
    user.cli(&[
        "patches",
        "review",
        patch_id.as_ref(),
        "--author",
        "reviewer",
        "--contents",
        "looks good",
        "--approve",
    ])
    .await?;

    // Wait for the sync_review_request_issues automation to process the review.
    {
        let client = harness.client()?;
        let rr_id = review_request.issue_id.clone();
        harness::wait_until(
            std::time::Duration::from_secs(5),
            std::time::Duration::from_millis(50),
            "ReviewRequest to be closed after review approval",
            || {
                let client = client.clone();
                let rr_id = rr_id.clone();
                async move {
                    let issue = client.get_issue(&rr_id, false).await.unwrap();
                    issue.issue.status == IssueStatus::Closed
                }
            },
        )
        .await?;
    }

    // 7. Verify ReviewRequest is closed (approval received).
    let review_request_updated = user.get_issue(&review_request.issue_id).await?;
    review_request_updated.assert_status(IssueStatus::Closed);

    // 8. Merge the patch by updating its status to Merged.
    // This simulates what happens when a PR is merged (either through the merge
    // queue or directly). The close_merge_request_issues automation fires on
    // the PatchUpdated event when the patch transitions to Merged.
    {
        let client = harness.client()?;
        let mut patch_record = client.get_patch(&patch_id).await?;
        patch_record.patch.status = PatchStatus::Merged;
        client
            .update_patch(&patch_id, &UpsertPatchRequest::new(patch_record.patch))
            .await?;
    }

    // Wait for close_merge_request_issues automation to process the merge.
    {
        let client = harness.client()?;
        let mr_id = merge_request.issue_id.clone();
        harness::wait_until(
            std::time::Duration::from_secs(5),
            std::time::Duration::from_millis(50),
            "MergeRequest to become terminal after patch merge",
            || {
                let client = client.clone();
                let mr_id = mr_id.clone();
                async move {
                    let issue = client.get_issue(&mr_id, false).await.unwrap();
                    matches!(
                        issue.issue.status,
                        IssueStatus::Closed | IssueStatus::Failed
                    )
                }
            },
        )
        .await?;
    }

    // 9. Verify final state.
    let merge_request_updated = user.get_issue(&merge_request.issue_id).await?;
    // MergeRequest should be closed after patch merge.
    assert!(
        matches!(
            merge_request_updated.issue.status,
            IssueStatus::Closed | IssueStatus::Failed
        ),
        "MergeRequest should be terminal after patch merge, got {:?}",
        merge_request_updated.issue.status
    );

    // 10. SWE's issue can now be closed (all children terminal).
    user.update_issue_status(&swe_issue_id, IssueStatus::Closed)
        .await
        .context("SWE's issue should be closable after all children are terminal")?;

    let swe_issue_final = user.get_issue(&swe_issue_id).await?;
    swe_issue_final.assert_status(IssueStatus::Closed);

    Ok(())
}

// ── Scenario 12: Automatic Backup Patches Are Excluded from Workflow ────

/// Backup patches (is_automatic_backup: true) must NOT trigger patch_workflow.
/// Normal patches DO trigger it.
#[tokio::test]
async fn backup_patches_do_not_trigger_patch_workflow() -> Result<()> {
    let repo = RepoName::from_str("acme/app")?;
    let pwc = PatchWorkflowConfig {
        review_requests: vec![ReviewRequestConfig {
            assignee: "reviewer".to_string(),
        }],
        merge_request: Some(MergeRequestConfig { assignee: None }),
        repos: Default::default(),
    };

    let harness = harness::TestHarness::builder()
        .with_repo("acme/app")
        .with_agent("swe", "Implement changes")
        .with_patch_workflow_config(pwc)
        .build()
        .await?;
    let user = harness.default_user();

    // Create an issue and spawn a task.
    let mut job_settings = JobSettings::default();
    job_settings.repo_name = Some(repo.clone());
    job_settings.image = Some("worker:latest".to_string());
    job_settings.branch = Some("main".to_string());

    let _swe_issue_id = user
        .create_issue_with_settings(
            "Backup patch test",
            IssueType::Task,
            IssueStatus::Open,
            Some("swe"),
            Some(job_settings),
        )
        .await?;

    let task_ids = harness.step_schedule().await?;
    assert_eq!(task_ids.len(), 1);

    // Create a backup patch directly via the API (is_automatic_backup: true).
    // Backup patches are typically created by the system when a task fails,
    // so we create it without a created_by task reference.
    let client = harness.client()?;
    let backup_patch = Patch::new(
        "Backup patch".to_string(),
        "Automatic backup".to_string(),
        String::new(),
        PatchStatus::Open,
        true, // is_automatic_backup = true
        None, // no created_by task
        Vec::new(),
        repo.clone(),
        None,
        false,
    );
    let backup_response = client
        .create_patch(&UpsertPatchRequest::new(backup_patch))
        .await?;
    let backup_patch_id = backup_response.patch_id;

    // Verify no ReviewRequest or MergeRequest issues were created.
    let all_issues = user.list_issues().await?;
    let workflow_issues: Vec<_> = all_issues
        .issues
        .iter()
        .filter(|i| {
            i.issue.issue_type == IssueType::ReviewRequest
                || i.issue.issue_type == IssueType::MergeRequest
        })
        .collect();
    assert!(
        workflow_issues.is_empty(),
        "backup patch should not trigger workflow, but found {} workflow issues",
        workflow_issues.len()
    );

    // Now create a normal patch via the worker (using the same task).
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

    // Verify ReviewRequest and MergeRequest ARE created for the normal patch.
    let all_issues_after = user.list_issues().await?;
    let workflow_issues_after: Vec<_> = all_issues_after
        .issues
        .iter()
        .filter(|i| {
            i.issue.issue_type == IssueType::ReviewRequest
                || i.issue.issue_type == IssueType::MergeRequest
        })
        .collect();
    assert!(
        workflow_issues_after.len() >= 2,
        "normal patch should trigger workflow, found {} workflow issues",
        workflow_issues_after.len()
    );

    // Verify the workflow issues reference the normal patch, not the backup.
    for wi in &workflow_issues_after {
        assert!(
            wi.issue.patches.contains(&normal_patch_id),
            "workflow issue should reference the normal patch"
        );
        assert!(
            !wi.issue.patches.contains(&backup_patch_id),
            "workflow issue should not reference the backup patch"
        );
    }

    Ok(())
}

// ── Scenario 13: Patch Closure Drops Review Workflow Issues ────

/// When a patch is closed without merging, its ReviewRequest issues should
/// be dropped and its MergeRequest issues should be failed.
#[tokio::test]
async fn closing_patch_drops_review_workflow_issues() -> Result<()> {
    let repo = RepoName::from_str("acme/app")?;
    let pwc = PatchWorkflowConfig {
        review_requests: vec![ReviewRequestConfig {
            assignee: "reviewer".to_string(),
        }],
        merge_request: Some(MergeRequestConfig { assignee: None }),
        repos: Default::default(),
    };

    let harness = harness::TestHarness::builder()
        .with_repo("acme/app")
        .with_agent("swe", "Implement changes")
        .with_patch_workflow_config(pwc)
        .build()
        .await?;
    let user = harness.default_user();

    // Create an issue and spawn a task.
    let mut job_settings = JobSettings::default();
    job_settings.repo_name = Some(repo.clone());
    job_settings.image = Some("worker:latest".to_string());
    job_settings.branch = Some("main".to_string());

    let _swe_issue_id = user
        .create_issue_with_settings(
            "Patch closure test",
            IssueType::Task,
            IssueStatus::Open,
            Some("swe"),
            Some(job_settings),
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
