mod harness;

use anyhow::{Context, Result};
use harness::test_job_settings_full;
use metis::client::MetisClientInterface;
use metis_common::{
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType, UpsertIssueRequest,
    },
    patches::{GithubPr, PatchStatus},
    IssueId, PatchId,
};
use metis_server::test_utils::{GitHubMockBuilder, MockPr, MockReview};
use std::str::FromStr;

/// Helper to create a merge-request tracking issue for a patch in tests.
async fn create_merge_request_issue(
    client: &dyn MetisClientInterface,
    patch_id: PatchId,
    assignee: String,
    parent_issue_id: IssueId,
    patch_title: String,
) -> Result<IssueId> {
    let parent_issue = client
        .get_issue(&parent_issue_id, false)
        .await
        .context("failed to fetch parent issue")?;
    let creator = parent_issue.issue.creator;
    let job_settings = parent_issue.issue.job_settings.clone();
    let description = format!("Review patch {}: {patch_title}", patch_id.as_ref());
    let issue = Issue::new(
        IssueType::MergeRequest,
        description,
        creator,
        String::new(),
        IssueStatus::Open,
        Some(assignee),
        Some(job_settings),
        Vec::new(),
        vec![IssueDependency::new(
            IssueDependencyType::ChildOf,
            parent_issue_id,
        )],
        vec![patch_id],
        false,
    );
    let response = client
        .create_issue(&UpsertIssueRequest::new(issue, None))
        .await
        .context("failed to create merge-request issue")?;
    Ok(response.issue_id)
}

#[tokio::test]
async fn sync_open_patches_closes_merge_request_issue_on_changes_requested() -> Result<()> {
    let pr_number = 99;
    let repo_owner = "octo";
    let repo_name = "repo";
    let review_branch = "feature/review";
    let repo = metis_common::RepoName::from_str("octo/repo")?;

    let mut harness = harness::TestHarness::builder()
        .with_repo("octo/repo")
        .with_github()
        .build()
        .await?;

    // Create a branch with a commit in the git remote.
    let head_sha = harness
        .remote("octo/repo")
        .create_branch(review_branch, "README.md", "base content\nreview change\n")
        .context("failed to create review branch")?;

    // Reconfigure GitHub mock with a PR that has a CHANGES_REQUESTED review.
    let (_github_server, github_app) = GitHubMockBuilder::new()
        .with_pr(
            repo_owner,
            repo_name,
            MockPr::new(pr_number, review_branch, &head_sha).with_review(
                MockReview::new("reviewer", "CHANGES_REQUESTED", "please update")
                    .with_author_id(1001),
            ),
        )
        .build()?;
    harness.state_mut().github_app = Some(github_app);

    // Create parent issue with job settings, patch with GitHub PR, and merge request issue.
    let user = harness.default_user();

    let parent_issue_id = user
        .create_issue_with_settings(
            "parent task",
            IssueType::Task,
            IssueStatus::Open,
            Some("requester"),
            Some(test_job_settings_full(&repo, "worker:latest", "main")),
        )
        .await?;

    let patch_id = user
        .create_patch_with_github(
            "Review patch",
            "Review description",
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

    let merge_request_issue_id = create_merge_request_issue(
        user.client(),
        patch_id.clone(),
        "requester".to_string(),
        parent_issue_id.clone(),
        "Review patch".to_string(),
    )
    .await?;

    // Run GitHub sync and verify outcomes.
    harness
        .step_github_sync()
        .await
        .context("sync_open_patches failed")?;

    let updated_patch = user.get_patch(&patch_id).await?;
    assert_eq!(updated_patch.patch.status, PatchStatus::ChangesRequested);
    assert!(updated_patch
        .patch
        .reviews
        .iter()
        .any(|review| review.author == "reviewer" && review.contents == "please update"));

    let merge_request_issue = user.get_issue(&merge_request_issue_id).await?;
    assert_eq!(merge_request_issue.issue.status, IssueStatus::Failed);

    let jobs = user.list_jobs_for_issue(&merge_request_issue_id).await?;
    assert!(
        jobs.is_empty(),
        "expected no followup job when merge request issue is failed"
    );

    Ok(())
}

#[tokio::test]
async fn sync_open_patches_closes_merge_request_issue_on_merged_pr() -> Result<()> {
    let pr_number = 100;
    let repo_owner = "octo";
    let repo_name = "repo";
    let merge_branch = "feature/merge";
    let repo = metis_common::RepoName::from_str("octo/repo")?;

    let mut harness = harness::TestHarness::builder()
        .with_repo("octo/repo")
        .with_github()
        .build()
        .await?;

    // Create a branch with a commit in the git remote.
    let head_sha = harness
        .remote("octo/repo")
        .create_branch(merge_branch, "README.md", "base content\nmerged change\n")
        .context("failed to create merge branch")?;

    // Reconfigure GitHub mock with a merged PR.
    let (_github_server, github_app) = GitHubMockBuilder::new()
        .with_pr(
            repo_owner,
            repo_name,
            MockPr::new(pr_number, merge_branch, &head_sha)
                .merged()
                .with_review(
                    MockReview::new("approver", "APPROVED", "looks good")
                        .with_id(201)
                        .with_author_id(2001),
                ),
        )
        .build()?;
    harness.state_mut().github_app = Some(github_app);

    let user = harness.default_user();

    let parent_issue_id = user
        .create_issue_with_settings(
            "parent task for merge",
            IssueType::Task,
            IssueStatus::Open,
            Some("requester"),
            Some(test_job_settings_full(&repo, "worker:latest", "main")),
        )
        .await?;

    let patch_id = user
        .create_patch_with_github(
            "Merge patch",
            "Merge description",
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

    let merge_request_issue_id = create_merge_request_issue(
        user.client(),
        patch_id.clone(),
        "requester".to_string(),
        parent_issue_id.clone(),
        "Merge patch".to_string(),
    )
    .await?;

    // Run GitHub sync and verify outcomes.
    harness
        .step_github_sync()
        .await
        .context("sync_open_patches failed")?;

    let updated_patch = user.get_patch(&patch_id).await?;
    assert_eq!(updated_patch.patch.status, PatchStatus::Merged);

    let merge_request_issue = user.get_issue(&merge_request_issue_id).await?;
    assert_eq!(merge_request_issue.issue.status, IssueStatus::Closed);

    Ok(())
}

#[tokio::test]
async fn sync_open_patches_fails_merge_request_issue_on_closed_pr() -> Result<()> {
    let pr_number = 100;
    let repo_owner = "octo";
    let repo_name = "repo";
    let closed_branch = "feature/closed";
    let repo = metis_common::RepoName::from_str("octo/repo")?;

    let mut harness = harness::TestHarness::builder()
        .with_repo("octo/repo")
        .with_github()
        .build()
        .await?;

    // Create a branch with a commit in the git remote.
    let head_sha = harness
        .remote("octo/repo")
        .create_branch(closed_branch, "README.md", "base content\nclosed change\n")
        .context("failed to create closed branch")?;

    // Reconfigure GitHub mock with a closed (not merged) PR.
    let (_github_server, github_app) = GitHubMockBuilder::new()
        .with_pr(
            repo_owner,
            repo_name,
            MockPr::new(pr_number, closed_branch, &head_sha)
                .closed()
                .with_review(
                    MockReview::new("commenter", "COMMENTED", "closing without merge")
                        .with_id(301)
                        .with_author_id(3001),
                ),
        )
        .build()?;
    harness.state_mut().github_app = Some(github_app);

    let user = harness.default_user();

    let parent_issue_id = user
        .create_issue_with_settings(
            "parent task for closed pr",
            IssueType::Task,
            IssueStatus::Open,
            Some("requester"),
            Some(test_job_settings_full(&repo, "worker:latest", "main")),
        )
        .await?;

    let patch_id = user
        .create_patch_with_github(
            "Closed patch",
            "Closed description",
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

    let merge_request_issue_id = create_merge_request_issue(
        user.client(),
        patch_id.clone(),
        "requester".to_string(),
        parent_issue_id.clone(),
        "Closed patch".to_string(),
    )
    .await?;

    // Run GitHub sync and verify outcomes.
    harness
        .step_github_sync()
        .await
        .context("sync_open_patches failed")?;

    let updated_patch = user.get_patch(&patch_id).await?;
    assert_eq!(updated_patch.patch.status, PatchStatus::Closed);

    let merge_request_issue = user.get_issue(&merge_request_issue_id).await?;
    assert_eq!(merge_request_issue.issue.status, IssueStatus::Failed);

    Ok(())
}
