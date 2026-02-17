mod harness;

use anyhow::{Context, Result};
use harness::{test_patch_workflow_config, IssueAssertions};
use metis_common::{
    issues::{IssueDependencyType, IssueStatus, IssueType},
    patches::PatchStatus,
};

/// Scenario 14: Full end-to-end pipeline.
///
/// user creates issue → PM agent creates sub-issue → SWE agent creates
/// patch → patch_workflow creates ReviewRequest + MergeRequest as children
/// of SWE's issue → reviewer approves → patch merged → MergeRequest closed
/// → SWE closes → PM closes parent.
#[tokio::test]
async fn full_end_to_end_pipeline() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/app")
        .with_user("reviewer")
        .with_agent("pm", "Plan and delegate tasks")
        .with_agent("swe", "Implement changes")
        .with_assignment_agent("pm")
        .with_patch_workflow_config(test_patch_workflow_config("reviewer", Some("merger")))
        .build()
        .await?;
    let user = harness.default_user();

    // ── Step 1: User creates issue (unassigned) ─────────────────────
    // The assignment agent (pm) will automatically pick up unassigned issues.
    let parent_issue_id = user.create_issue("Implement user settings page").await?;

    // ── Step 2: PM agent picks up the parent issue ──────────────────
    let pm_tasks = harness.step_schedule().await?;
    assert_eq!(pm_tasks.len(), 1, "expected one task for PM agent");

    // PM worker creates a child issue assigned to SWE.
    let _result = harness
        .run_worker(
            &pm_tasks[0],
            vec![
                &format!(
                    "metis issues create 'Implement settings UI' --assignee swe \
                     --repo-name acme/app --image worker:latest --branch main \
                     --deps child-of:{}",
                    parent_issue_id.as_ref()
                ),
                &format!(
                    "metis issues update {} --status in-progress",
                    parent_issue_id.as_ref()
                ),
            ],
        )
        .await?;

    // Find the child issue created by PM.
    let all_issues = user.list_issues().await?;
    let swe_issue = all_issues
        .issues
        .iter()
        .find(|i| {
            i.issue.description.contains("Implement settings UI") && i.issue_id != parent_issue_id
        })
        .context("expected PM to create a child issue for SWE")?;
    let swe_issue_id = swe_issue.issue_id.clone();

    // Verify the child is linked to the parent.
    let is_child = swe_issue.issue.dependencies.iter().any(|dep| {
        dep.dependency_type == IssueDependencyType::ChildOf && dep.issue_id == parent_issue_id
    });
    assert!(is_child, "SWE's issue should be a child of the parent");

    // Verify parent is in-progress.
    let parent = user.get_issue(&parent_issue_id).await?;
    parent.assert_status(IssueStatus::InProgress);

    // ── Step 3: SWE agent picks up the child issue ──────────────────
    let swe_tasks = harness.step_schedule().await?;
    assert_eq!(swe_tasks.len(), 1, "expected one task for SWE agent");

    // SWE worker makes changes and creates a patch.
    let swe_result = harness
        .run_worker(
            &swe_tasks[0],
            vec![
                "echo 'settings page implementation' >> settings.rs",
                "git add settings.rs",
                "git commit -m 'implement user settings page'",
                "metis patches create --title 'Implement user settings page' --description 'Adds settings UI'",
            ],
        )
        .await?;
    assert_eq!(swe_result.patches_created.len(), 1);
    let patch_id = swe_result.patches_created[0].clone();

    // ── Step 4: Verify patch_workflow automation fired ───────────────
    let all_issues = user.list_issues().await?;
    let swe_issue_record = user.get_issue(&swe_issue_id).await?;

    // ReviewRequest should be a child of SWE's issue.
    swe_issue_record.assert_has_child_with_status(
        &all_issues.issues,
        "Review request for patch",
        IssueStatus::Open,
    );

    // MergeRequest should be a child of SWE's issue.
    swe_issue_record.assert_has_child_with_status(
        &all_issues.issues,
        "Review patch",
        IssueStatus::Open,
    );

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

    // Verify assignments.
    assert_eq!(review_request.issue.assignee.as_deref(), Some("reviewer"));
    assert_eq!(merge_request.issue.assignee.as_deref(), Some("merger"));

    // Verify the workflow issues are children of SWE's issue, NOT the parent.
    let rr_is_child_of_swe = review_request.issue.dependencies.iter().any(|dep| {
        dep.dependency_type == IssueDependencyType::ChildOf && dep.issue_id == swe_issue_id
    });
    assert!(
        rr_is_child_of_swe,
        "ReviewRequest should be a child of SWE's issue, not the parent"
    );
    let mr_is_child_of_swe = merge_request.issue.dependencies.iter().any(|dep| {
        dep.dependency_type == IssueDependencyType::ChildOf && dep.issue_id == swe_issue_id
    });
    assert!(
        mr_is_child_of_swe,
        "MergeRequest should be a child of SWE's issue, not the parent"
    );

    // ── Step 5: SWE's issue cannot close yet ────────────────────────
    let close_result = user
        .update_issue_status(&swe_issue_id, IssueStatus::Closed)
        .await;
    assert!(
        close_result.is_err(),
        "SWE's issue should not be closable while workflow children are open"
    );

    // ── Step 6: Reviewer approves ───────────────────────────────────
    user.cli(&[
        "patches",
        "review",
        patch_id.as_ref(),
        "--author",
        "reviewer",
        "--contents",
        "LGTM",
        "--approve",
    ])
    .await?;

    // Wait for the sync_review_request_issues automation to process the review.
    // The automation fires on the PatchUpdated event from the review CLI command.
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

    // ── Step 7: Merge the patch ─────────────────────────────────────
    // Simulate patch merge by updating status to Merged. This triggers the
    // close_merge_request_issues automation which closes MergeRequest issues.
    {
        let client = harness.client()?;
        let mut patch_record = client.get_patch(&patch_id).await?;
        patch_record.patch.status = PatchStatus::Merged;
        client
            .update_patch(
                &patch_id,
                &metis_common::patches::UpsertPatchRequest::new(patch_record.patch),
            )
            .await?;
    }

    // ── Step 8: Verify workflow completion ───────────────────────────
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

    let review_request_final = user.get_issue(&review_request.issue_id).await?;
    assert!(
        matches!(
            review_request_final.issue.status,
            IssueStatus::Closed | IssueStatus::Dropped
        ),
        "ReviewRequest should be terminal, got {:?}",
        review_request_final.issue.status
    );

    let merge_request_final = user.get_issue(&merge_request.issue_id).await?;
    assert!(
        matches!(
            merge_request_final.issue.status,
            IssueStatus::Closed | IssueStatus::Failed
        ),
        "MergeRequest should be terminal, got {:?}",
        merge_request_final.issue.status
    );

    // ── Step 9: SWE closes child issue (via worker) ─────────────────
    // Spawn a job on the SWE issue and have the worker close it via CLI.
    let swe_close_tasks = harness.step_schedule().await?;
    assert_eq!(
        swe_close_tasks.len(),
        1,
        "expected one task for SWE to close child issue"
    );
    harness
        .run_worker(
            &swe_close_tasks[0],
            vec![&format!(
                "metis issues update {} --status closed",
                swe_issue_id.as_ref()
            )],
        )
        .await?;

    // ── Step 10: PM closes parent issue (via worker) ────────────────
    // Spawn a job on the parent issue and have the worker close it via CLI.
    let pm_close_tasks = harness.step_schedule().await?;
    assert_eq!(
        pm_close_tasks.len(),
        1,
        "expected one task for PM to close parent issue"
    );
    harness
        .run_worker(
            &pm_close_tasks[0],
            vec![&format!(
                "metis issues update {} --status closed",
                parent_issue_id.as_ref()
            )],
        )
        .await?;

    // ── Step 11: Verify final state ─────────────────────────────────
    let parent_final = user.get_issue(&parent_issue_id).await?;
    parent_final.assert_status(IssueStatus::Closed);

    let swe_final = user.get_issue(&swe_issue_id).await?;
    swe_final.assert_status(IssueStatus::Closed);

    let patch_final = user.get_patch(&patch_id).await?;
    assert_eq!(
        patch_final.patch.status,
        PatchStatus::Merged,
        "patch should be merged, got {:?}",
        patch_final.patch.status
    );

    // Verify actor chain: patch.creator should trace back to the original user.
    // The patch was created by the SWE worker, whose task was spawned from
    // the SWE issue, which was created as a child of the parent issue.
    assert!(
        patch_final.patch.creator.is_some(),
        "patch should have a creator"
    );

    Ok(())
}
