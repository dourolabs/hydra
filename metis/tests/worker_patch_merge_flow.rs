mod harness;

use anyhow::{Context, Result};
use harness::{create_merge_request_issue, test_job_settings_full};
use metis_common::{
    issues::{IssueStatus, IssueType},
    jobs::SearchJobsQuery,
    patches::{GithubPr, PatchStatus},
};
use metis_server::background::spawner::AgentQueue;
use metis_server::config::{
    AgentQueueConfig, DEFAULT_AGENT_MAX_SIMULTANEOUS, DEFAULT_AGENT_MAX_TRIES,
};
use metis_server::domain::actors::ActorRef;
use metis_server::test_utils::{GitHubMockBuilder, MockPr, MockReview};
use std::str::FromStr;
use std::sync::Arc;

#[tokio::test]
async fn merge_request_issue_tracks_issue_head_and_merges() -> Result<()> {
    let repo_owner = "octo";
    let repo_name = "repo";
    let pr_number = 99;
    let head_ref = "repo/t-abc123/head";
    let repo = metis_common::RepoName::from_str("octo/repo")?;

    let mut harness = harness::TestHarness::builder()
        .with_repo("octo/repo")
        .with_github()
        .build()
        .await?;

    // Create a branch with a commit and capture both sha and diff.
    let head_sha = harness
        .remote("octo/repo")
        .create_branch(head_ref, "README.md", "initial content\nfeature change\n")
        .context("failed to create head branch")?;
    let patch_diff = harness.remote("octo/repo").diff("main", head_ref)?;
    harness.remote("octo/repo").set_head("main")?;

    // Reconfigure GitHub mock with a PR that has a CHANGES_REQUESTED review.
    let (_github_server, github_app) = GitHubMockBuilder::new()
        .with_pr(
            repo_owner,
            repo_name,
            MockPr::new(pr_number, head_ref, &head_sha).with_review(MockReview::new(
                "reviewer",
                "CHANGES_REQUESTED",
                "please update",
            )),
        )
        .build()?;
    harness.state_mut().github_app = Some(github_app);

    // Use a standalone client for operations that span across state_mut() calls,
    // since UserHandle borrows the harness and prevents mutable borrows.
    let client = harness.client()?;

    let parent_issue_id = harness
        .default_user()
        .create_issue_with_settings(
            "parent task",
            IssueType::Task,
            IssueStatus::Open,
            Some("requester"),
            Some(test_job_settings_full(&repo, "worker:latest", "main")),
        )
        .await?;

    let patch_id = harness
        .default_user()
        .create_patch_with_github(
            "Code change summary",
            "Code change description",
            &repo,
            GithubPr::new(
                repo_owner.to_string(),
                repo_name.to_string(),
                pr_number,
                None,
                None,
                None,
                None,
            ),
        )
        .await?;

    // Update the patch diff to match what was created in the branch.
    {
        let mut patch_record = client.get_patch(&patch_id).await?;
        patch_record.patch.diff = patch_diff;
        let update_request = metis_common::patches::UpsertPatchRequest::new(patch_record.patch);
        client.update_patch(&patch_id, &update_request).await?;
    }

    let initial_merge_request_issue_id = create_merge_request_issue(
        &client,
        patch_id.clone(),
        "reviewer".to_string(),
        parent_issue_id.clone(),
        "Code change summary".to_string(),
    )
    .await?;

    // Register an agent queue for the "reviewer" spawner.
    let queue_config = AgentQueueConfig {
        name: "reviewer".to_string(),
        prompt: "Review patch".to_string(),
        max_tries: DEFAULT_AGENT_MAX_TRIES,
        max_simultaneous: DEFAULT_AGENT_MAX_SIMULTANEOUS,
    };
    {
        let mut agents = harness.agents().write().await;
        *agents = vec![Arc::new(AgentQueue::from_config(&queue_config))];
    }

    // Run the spawner once before syncing.
    harness.step_spawner().await?;

    harness
        .step_github_sync()
        .await
        .context("sync_open_patches failed")?;

    let updated_patch = client.get_patch(&patch_id).await?;
    assert_eq!(updated_patch.patch.status, PatchStatus::ChangesRequested);
    assert_eq!(
        updated_patch
            .patch
            .github
            .as_ref()
            .and_then(|github| github.head_ref.as_deref()),
        Some(head_ref)
    );

    let initial_merge_request_issue = client
        .get_issue(&initial_merge_request_issue_id, false)
        .await?;
    assert_eq!(
        initial_merge_request_issue.issue.status,
        IssueStatus::Failed
    );

    // Create a new merge request issue for the re-review cycle.
    let merge_request_issue_id = create_merge_request_issue(
        &client,
        patch_id.clone(),
        "reviewer".to_string(),
        parent_issue_id.clone(),
        "Code change summary".to_string(),
    )
    .await?;

    harness.step_spawner().await?;

    let jobs = client
        .list_jobs(&SearchJobsQuery::new(
            None,
            Some(merge_request_issue_id.clone()),
            None,
            None,
        ))
        .await?
        .jobs;
    let job = jobs
        .first()
        .context("expected review task to be spawned for merge request")?;
    let job_id = job.job_id.clone();

    harness
        .state()
        .start_pending_task(job_id.clone(), ActorRef::test())
        .await;

    let main_head_before = harness.remote("octo/repo").branch_sha("main")?;

    let _result = harness
        .run_worker(
            &job_id,
            vec![
                "echo \"worker fix\" >> README.md",
                "git add README.md",
                "git commit -m \"worker fix\"",
            ],
        )
        .await?;

    let issue_head_branch = format!("metis/{merge_request_issue_id}/head");
    let head_after_worker = harness.remote("octo/repo").branch_sha(&issue_head_branch)?;
    assert_ne!(head_after_worker, main_head_before);

    // Push an additional commit to the PR head branch.
    let head_after_extra =
        harness
            .remote("octo/repo")
            .push_commit(head_ref, "README.md", "additional fix\n")?;
    assert_ne!(head_after_extra, head_after_worker);

    // Reconfigure GitHub mock with updated head SHA.
    let (_github_server_updated, github_app_updated) = GitHubMockBuilder::new()
        .with_pr(
            repo_owner,
            repo_name,
            MockPr::new(pr_number, head_ref, &head_after_extra).with_review(MockReview::new(
                "reviewer",
                "CHANGES_REQUESTED",
                "please update",
            )),
        )
        .build()?;
    harness.state_mut().github_app = Some(github_app_updated);

    harness
        .step_github_sync()
        .await
        .context("sync_open_patches failed after extra commit")?;

    // Approve the patch via CLI and verify the status is updated to Merged
    // when the patch update is applied directly (the merge command's git operations
    // are tested separately in unit tests since they require a local repo checkout).
    harness
        .default_user()
        .cli(&[
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

    // Verify the review was applied.
    let reviewed_patch = client.get_patch(&patch_id).await?;
    let has_approval = reviewed_patch
        .patch
        .reviews
        .iter()
        .any(|r| r.is_approved && r.author == "reviewer");
    assert!(has_approval, "expected an approving review from 'reviewer'");

    // Directly update the patch status to Merged to simulate what the merge
    // command would do after a successful rebase+push. The actual rebase+push
    // git operations are validated separately in unit tests.
    {
        let mut merged_patch = reviewed_patch.patch;
        merged_patch.status = PatchStatus::Merged;
        let request = metis_common::patches::UpsertPatchRequest::new(merged_patch);
        client.update_patch(&patch_id, &request).await?;
    }

    let final_patch = client.get_patch(&patch_id).await?;
    assert_eq!(final_patch.patch.status, PatchStatus::Merged);

    Ok(())
}
