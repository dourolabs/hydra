//! Scenario 3: Review rejection and re-review cycle.
//!
//! Verifies the complete lifecycle:
//! 1. SWE creates patch via worker → patch_workflow creates ReviewRequest + MergeRequest
//! 2. Reviewer submits non-approving review (changes requested)
//! 3. step_github_sync() → patch status → ChangesRequested
//! 4. close_merge_request_issues fails MergeRequest; sync_review_request_issues fails ReviewRequest
//! 5. Patch re-opened (ChangesRequested → Open)
//! 6. patch_workflow creates new ReviewRequest + MergeRequest
//! 7. Reviewer approves → patch merged → all workflow issues terminal
//! 8. Old (Failed) and new (Closed/Dropped) workflow issues coexist

mod harness;

use anyhow::{Context, Result};
use harness::{test_job_settings_full, test_patch_workflow_config};
use metis::client::MetisClientInterface;
use metis_common::{
    issues::{IssueDependencyType, IssueStatus, IssueType, SearchIssuesQuery},
    patches::{GithubPr, PatchStatus, UpsertPatchRequest},
    IssueId, RepoName,
};
use metis_server::test_utils::{GitHubMockBuilder, MockPr, MockReview};
use std::str::FromStr;

/// Find all child issues of a given parent that match a specific type.
async fn find_children_by_type(
    client: &dyn MetisClientInterface,
    parent_id: &IssueId,
    issue_type: IssueType,
) -> Result<Vec<(IssueId, metis_common::issues::IssueVersionRecord)>> {
    let all_issues = client
        .list_issues(&SearchIssuesQuery::default())
        .await
        .context("failed to list issues")?;

    let mut matches = Vec::new();
    for record in all_issues.issues {
        if record.issue.issue_type != issue_type {
            continue;
        }
        let is_child =
            record.issue.dependencies.iter().any(|d| {
                d.dependency_type == IssueDependencyType::ChildOf && d.issue_id == *parent_id
            });
        if is_child {
            matches.push((record.issue_id.clone(), record));
        }
    }
    Ok(matches)
}

/// Find child issues matching a type and status.
async fn find_children_by_type_and_status(
    client: &dyn MetisClientInterface,
    parent_id: &IssueId,
    issue_type: IssueType,
    status: IssueStatus,
) -> Result<Vec<(IssueId, metis_common::issues::IssueVersionRecord)>> {
    let children = find_children_by_type(client, parent_id, issue_type).await?;
    Ok(children
        .into_iter()
        .filter(|(_, r)| r.issue.status == status)
        .collect())
}

#[tokio::test]
async fn review_rejection_then_approve_merge_cycle() -> Result<()> {
    let pr_number = 42;
    let repo_owner = "test-org";
    let repo_name = "review-repo";
    let head_ref = "feature/swe-work";
    let repo = RepoName::from_str("test-org/review-repo")?;

    let mut harness = harness::TestHarness::builder()
        .with_repo("test-org/review-repo")
        .with_github()
        .with_user("reviewer")
        .with_agent("swe", "You are a software engineer")
        .with_patch_workflow_config(test_patch_workflow_config("reviewer", Some("swe")))
        .build()
        .await?;

    // Use a standalone client for API operations that span state_mut() calls.
    let client = harness.client()?;

    // ── Step 1: Create SWE's issue with job settings ──────────────
    let job_settings = test_job_settings_full(&repo, "worker:latest", "main");

    let swe_issue_id = harness
        .default_user()
        .create_issue_with_settings(
            "SWE task: implement feature",
            IssueType::Task,
            IssueStatus::Open,
            Some("swe"),
            Some(job_settings.clone()),
        )
        .await?;

    // ── Step 2: Spawn a task for the SWE issue and run the worker ─
    // The worker creates a patch via `metis patches create`, which sets up
    // the created_by chain that patch_workflow uses to discover the parent issue.
    let task_ids = harness.step_schedule().await?;
    assert_eq!(
        task_ids.len(),
        1,
        "should spawn exactly one task for the SWE issue"
    );
    let swe_task_id = &task_ids[0];

    let result = harness
        .run_worker(
            swe_task_id,
            vec![
                "echo 'fn main() { /* v1 */ }' > feature.rs",
                "git add feature.rs",
                "git commit -m 'implement feature v1'",
                "metis patches create --title 'Implement feature' --description 'First attempt'",
            ],
        )
        .await?;

    assert_eq!(
        result.patches_created.len(),
        1,
        "worker should create exactly one patch"
    );
    let patch_id = result.patches_created[0].clone();

    // ── Step 3: Verify patch_workflow created ReviewRequest + MergeRequest ──
    let rr_children =
        find_children_by_type(&client, &swe_issue_id, IssueType::ReviewRequest).await?;
    let mr_children =
        find_children_by_type(&client, &swe_issue_id, IssueType::MergeRequest).await?;

    assert_eq!(
        rr_children.len(),
        1,
        "patch_workflow should create 1 ReviewRequest issue"
    );
    assert_eq!(
        mr_children.len(),
        1,
        "patch_workflow should create 1 MergeRequest issue"
    );

    let (old_rr_id, old_rr) = &rr_children[0];
    let (old_mr_id, old_mr) = &mr_children[0];

    assert_eq!(old_rr.issue.status, IssueStatus::Open);
    assert_eq!(old_mr.issue.status, IssueStatus::Open);
    assert_eq!(old_rr.issue.assignee, Some("reviewer".to_string()));
    assert_eq!(old_mr.issue.assignee, Some("swe".to_string()));

    // Verify MergeRequest is blocked-on ReviewRequest
    let mr_blocked_on: Vec<_> = old_mr
        .issue
        .dependencies
        .iter()
        .filter(|d| d.dependency_type == IssueDependencyType::BlockedOn)
        .map(|d| d.issue_id.clone())
        .collect();
    assert!(
        mr_blocked_on.contains(old_rr_id),
        "MergeRequest should be blocked-on ReviewRequest"
    );

    // ── Step 4: Set up GitHub mock with CHANGES_REQUESTED review ──
    // We need to add GitHub PR metadata to the patch first.
    let patch_branch = {
        let patch = client.get_patch(&patch_id).await?;
        patch
            .patch
            .branch_name
            .clone()
            .unwrap_or_else(|| head_ref.to_string())
    };

    // Create the branch in the git remote that matches the patch branch.
    let head_sha = harness
        .remote("test-org/review-repo")
        .create_branch(&patch_branch, "feature.rs", "fn main() { /* v1 */ }\n")
        .unwrap_or_else(|_| {
            // Branch may already exist from worker, get its SHA
            harness
                .remote("test-org/review-repo")
                .branch_sha(&patch_branch)
                .expect("branch should exist")
        });

    // Add GitHub PR metadata to the patch
    {
        let mut patch_record = client.get_patch(&patch_id).await?;
        patch_record.patch.github = Some(GithubPr::new(
            repo_owner.to_string(),
            repo_name.to_string(),
            pr_number,
            None,
            None,
            None,
            None,
        ));
        let request = UpsertPatchRequest::new(patch_record.patch);
        client.update_patch(&patch_id, &request).await?;
    }

    // Configure GitHub mock with CHANGES_REQUESTED review
    let (_github_server, github_app) = GitHubMockBuilder::new()
        .with_pr(
            repo_owner,
            repo_name,
            MockPr::new(pr_number, &patch_branch, &head_sha).with_review(
                MockReview::new(
                    "reviewer",
                    "CHANGES_REQUESTED",
                    "Please fix the implementation",
                )
                .with_author_id(1001),
            ),
        )
        .build()?;
    harness.state_mut().github_app = Some(github_app);

    // ── Step 5: step_github_sync() processes the review ───────────
    harness
        .step_github_sync()
        .await
        .context("step_github_sync failed after CHANGES_REQUESTED")?;

    // ── Step 6: Verify patch status → ChangesRequested ────────────
    let patch_after_reject = client.get_patch(&patch_id).await?;
    assert_eq!(
        patch_after_reject.patch.status,
        PatchStatus::ChangesRequested,
        "patch should be ChangesRequested after non-approving review"
    );

    // ── Step 7: Verify old MergeRequest → Failed ──────────────────
    let old_mr_after = client.get_issue(old_mr_id, false).await?;
    assert_eq!(
        old_mr_after.issue.status,
        IssueStatus::Failed,
        "old MergeRequest should be Failed after ChangesRequested"
    );

    // ── Step 8: Verify old ReviewRequest → Failed ─────────────────
    let old_rr_after = client.get_issue(old_rr_id, false).await?;
    assert_eq!(
        old_rr_after.issue.status,
        IssueStatus::Failed,
        "old ReviewRequest should be Failed after non-approving review"
    );

    // ── Step 9: Re-open the patch via a worker (ChangesRequested → Open) ──
    // The background spawner automatically creates a new task for the SWE issue
    // since the previous task completed. The worker then makes fixes and re-opens
    // the patch via CLI.
    {
        let task_ids = harness.step_schedule().await?;
        assert_eq!(
            task_ids.len(),
            1,
            "should spawn exactly one new task for the SWE issue"
        );
        let swe_task_id_2 = &task_ids[0];

        let patch_id_str = patch_id.as_ref();
        harness
            .run_worker(
                swe_task_id_2,
                vec![
                    "echo 'fn main() { /* v2 - fixed */ }' > feature.rs",
                    "git add feature.rs",
                    "git commit -m 'address review feedback'",
                    &format!("metis patches update {patch_id_str} --status Open"),
                ],
            )
            .await?;
    }

    // ── Step 10: Verify patch_workflow re-fires → new workflow issues ──
    let new_rr_children = find_children_by_type_and_status(
        &client,
        &swe_issue_id,
        IssueType::ReviewRequest,
        IssueStatus::Open,
    )
    .await?;
    let new_mr_children = find_children_by_type_and_status(
        &client,
        &swe_issue_id,
        IssueType::MergeRequest,
        IssueStatus::Open,
    )
    .await?;

    assert_eq!(
        new_rr_children.len(),
        1,
        "patch_workflow should create 1 new ReviewRequest issue after re-open"
    );
    assert_eq!(
        new_mr_children.len(),
        1,
        "patch_workflow should create 1 new MergeRequest issue after re-open"
    );

    let (new_rr_id, _) = &new_rr_children[0];
    let (new_mr_id, _) = &new_mr_children[0];

    assert_ne!(
        new_rr_id, old_rr_id,
        "new ReviewRequest should be a different issue"
    );
    assert_ne!(
        new_mr_id, old_mr_id,
        "new MergeRequest should be a different issue"
    );

    // ── Step 11: Reviewer approves via CLI ────────────────────────
    harness
        .default_user()
        .cli(&[
            "patches",
            "review",
            patch_id.as_ref(),
            "--author",
            "reviewer",
            "--contents",
            "LGTM, approved",
            "--approve",
        ])
        .await?;

    // ── Step 12: Reconfigure GitHub mock with APPROVED + merged PR ─
    let (_github_server2, github_app2) = GitHubMockBuilder::new()
        .with_pr(
            repo_owner,
            repo_name,
            MockPr::new(pr_number, &patch_branch, &head_sha)
                .merged()
                .with_review(
                    MockReview::new("reviewer", "APPROVED", "LGTM, approved").with_author_id(1001),
                ),
        )
        .build()?;
    harness.state_mut().github_app = Some(github_app2);

    // ── Step 13: step_github_sync() processes approval + merge ────
    harness
        .step_github_sync()
        .await
        .context("step_github_sync failed after APPROVED")?;

    // ── Step 14: Verify patch → Merged ────────────────────────────
    let patch_final = client.get_patch(&patch_id).await?;
    assert_eq!(
        patch_final.patch.status,
        PatchStatus::Merged,
        "patch should be Merged after approval and merge"
    );

    // ── Step 15: Verify new ReviewRequest → Closed or Dropped ─────
    let new_rr_final = client.get_issue(new_rr_id, false).await?;
    assert!(
        matches!(
            new_rr_final.issue.status,
            IssueStatus::Closed | IssueStatus::Dropped
        ),
        "new ReviewRequest should be Closed or Dropped after merge, got {:?}",
        new_rr_final.issue.status
    );

    // ── Step 16: Verify new MergeRequest → Closed ─────────────────
    let new_mr_final = client.get_issue(new_mr_id, false).await?;
    assert_eq!(
        new_mr_final.issue.status,
        IssueStatus::Closed,
        "new MergeRequest should be Closed after merge"
    );

    // ── Step 17: Verify coexistence of old and new workflow issues ─
    let all_rr = find_children_by_type(&client, &swe_issue_id, IssueType::ReviewRequest).await?;
    let all_mr = find_children_by_type(&client, &swe_issue_id, IssueType::MergeRequest).await?;

    assert_eq!(
        all_rr.len(),
        2,
        "should have 2 ReviewRequest issues (old Failed + new terminal)"
    );
    assert_eq!(
        all_mr.len(),
        2,
        "should have 2 MergeRequest issues (old Failed + new Closed)"
    );

    // Verify old ones are still Failed
    let old_rr_check = client.get_issue(old_rr_id, false).await?;
    assert_eq!(old_rr_check.issue.status, IssueStatus::Failed);
    let old_mr_check = client.get_issue(old_mr_id, false).await?;
    assert_eq!(old_mr_check.issue.status, IssueStatus::Failed);

    // All workflow children should be in terminal states
    for (id, record) in all_rr.iter().chain(all_mr.iter()) {
        assert!(
            matches!(
                record.issue.status,
                IssueStatus::Closed | IssueStatus::Failed | IssueStatus::Dropped
            ),
            "workflow issue {id} should be terminal, got {:?}",
            record.issue.status
        );
    }

    Ok(())
}
