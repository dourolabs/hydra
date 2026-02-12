use anyhow::{Context, Result};
use metis::command::patches::create_merge_request_issue;
use metis_common::{
    issues::{IssueStatus, IssueType, JobSettings},
    jobs::SearchJobsQuery,
    patches::{GithubPr, PatchStatus},
};
use metis_server::test_utils::{GitHubMockBuilder, MockPr, MockReview};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

mod common;

use common::test_helpers::init_test_server_with_remote_and_github;

fn git_output(args: &[&str], cwd: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("failed to run git {args:?}"))?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn create_branch_with_commit(remote_url: &str, branch: &str, line: &str) -> Result<String> {
    let tempdir = TempDir::new().context("failed to create tempdir for branch setup")?;
    let repo_path = tempdir.path();

    git_output(&["clone", remote_url, "."], repo_path)?;
    git_output(&["config", "user.name", "Test User"], repo_path)?;
    git_output(&["config", "user.email", "test@example.com"], repo_path)?;
    git_output(&["checkout", "-b", branch], repo_path)?;
    std::fs::write(repo_path.join("README.md"), format!("base content\n{line}"))
        .context("failed to write README content")?;
    git_output(&["add", "README.md"], repo_path)?;
    git_output(&["commit", "-m", "feature change"], repo_path)?;
    git_output(&["push", "-u", "origin", branch], repo_path)?;

    git_output(&["rev-parse", "HEAD"], repo_path)
}

#[tokio::test]
async fn sync_open_patches_closes_merge_request_issue_on_changes_requested() -> Result<()> {
    let pr_number = 99;
    let repo_owner = "octo";
    let repo_name = "repo";
    let review_branch = "feature/review";

    let (_github_server, github_app) = GitHubMockBuilder::new()
        .with_installation(repo_owner, repo_name)
        .build()?;

    let mut env = init_test_server_with_remote_and_github("octo/repo", Some(github_app)).await?;
    let repository = env
        .state
        .repository_from_store(&env.service_repo_name)
        .await
        .context("failed to load service repository config")?;
    let head_sha =
        create_branch_with_commit(&repository.remote_url, review_branch, "review change\n")
            .context("failed to create review branch")?;

    // Replace with mocks that include the PR and review
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
    env.state.github_app = Some(github_app);

    let mut job_settings = JobSettings::default();
    job_settings.repo_name = Some(env.service_repo_name.clone());
    job_settings.image = Some("worker:latest".to_string());
    job_settings.branch = Some("main".to_string());

    let parent_issue_id = env
        .create_issue(
            "parent task",
            IssueType::Task,
            IssueStatus::Open,
            Some("requester".to_string()),
            Some(job_settings.clone()),
        )
        .await?;

    let patch_id = env
        .create_patch(
            "Review patch",
            "Review description",
            "diff",
            PatchStatus::Open,
            Some(GithubPr::new(
                repo_owner.to_string(),
                repo_name.to_string(),
                pr_number,
                None,
                None,
                None,
                None,
            )),
            None,
        )
        .await?;

    let merge_request_issue_id = create_merge_request_issue(
        &env.client,
        patch_id.clone(),
        "requester".to_string(),
        parent_issue_id.clone(),
        "Review patch".to_string(),
        "Review description".to_string(),
    )
    .await?
    .id;

    env.run_github_sync(60)
        .await
        .context("sync_open_patches failed")?;

    let updated_patch = env.client.get_patch(&patch_id).await?.patch;
    assert_eq!(updated_patch.status, PatchStatus::ChangesRequested);
    assert!(updated_patch
        .reviews
        .iter()
        .any(|review| review.author == "reviewer" && review.contents == "please update"));

    let merge_request_issue = env.client.get_issue(&merge_request_issue_id).await?.issue;
    assert_eq!(merge_request_issue.status, IssueStatus::Failed);

    let jobs = env
        .client
        .list_jobs(&SearchJobsQuery::new(
            None,
            Some(merge_request_issue_id.clone()),
            None,
        ))
        .await?
        .jobs;
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

    let (_github_server, github_app) = GitHubMockBuilder::new()
        .with_installation(repo_owner, repo_name)
        .build()?;

    let mut env = init_test_server_with_remote_and_github("octo/repo", Some(github_app)).await?;
    let repository = env
        .state
        .repository_from_store(&env.service_repo_name)
        .await
        .context("failed to load service repository config")?;
    let head_sha =
        create_branch_with_commit(&repository.remote_url, merge_branch, "merged change\n")
            .context("failed to create merge branch")?;

    // Replace with mocks that include the merged PR
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
    env.state.github_app = Some(github_app);

    let mut job_settings = JobSettings::default();
    job_settings.repo_name = Some(env.service_repo_name.clone());
    job_settings.image = Some("worker:latest".to_string());
    job_settings.branch = Some("main".to_string());

    let parent_issue_id = env
        .create_issue(
            "parent task for merge",
            IssueType::Task,
            IssueStatus::Open,
            Some("requester".to_string()),
            Some(job_settings.clone()),
        )
        .await?;

    let patch_id = env
        .create_patch(
            "Merge patch",
            "Merge description",
            "diff",
            PatchStatus::Open,
            Some(GithubPr::new(
                repo_owner.to_string(),
                repo_name.to_string(),
                pr_number,
                None,
                None,
                None,
                None,
            )),
            None,
        )
        .await?;

    let merge_request_issue_id = create_merge_request_issue(
        &env.client,
        patch_id.clone(),
        "requester".to_string(),
        parent_issue_id.clone(),
        "Merge patch".to_string(),
        "Merge description".to_string(),
    )
    .await?
    .id;

    env.run_github_sync(60)
        .await
        .context("sync_open_patches failed")?;

    let updated_patch = env.client.get_patch(&patch_id).await?.patch;
    assert_eq!(updated_patch.status, PatchStatus::Merged);

    let merge_request_issue = env.client.get_issue(&merge_request_issue_id).await?.issue;
    assert_eq!(merge_request_issue.status, IssueStatus::Closed);

    Ok(())
}

#[tokio::test]
async fn sync_open_patches_fails_merge_request_issue_on_closed_pr() -> Result<()> {
    let pr_number = 100;
    let repo_owner = "octo";
    let repo_name = "repo";
    let closed_branch = "feature/closed";

    let (_github_server, github_app) = GitHubMockBuilder::new()
        .with_installation(repo_owner, repo_name)
        .build()?;

    let mut env = init_test_server_with_remote_and_github("octo/repo", Some(github_app)).await?;
    let repository = env
        .state
        .repository_from_store(&env.service_repo_name)
        .await
        .context("failed to load service repository config")?;
    let head_sha =
        create_branch_with_commit(&repository.remote_url, closed_branch, "closed change\n")
            .context("failed to create closed branch")?;

    // Replace with mocks that include the closed (not merged) PR
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
    env.state.github_app = Some(github_app);

    let mut job_settings = JobSettings::default();
    job_settings.repo_name = Some(env.service_repo_name.clone());
    job_settings.image = Some("worker:latest".to_string());
    job_settings.branch = Some("main".to_string());

    let parent_issue_id = env
        .create_issue(
            "parent task for closed pr",
            IssueType::Task,
            IssueStatus::Open,
            Some("requester".to_string()),
            Some(job_settings.clone()),
        )
        .await?;

    let patch_id = env
        .create_patch(
            "Closed patch",
            "Closed description",
            "diff",
            PatchStatus::Open,
            Some(GithubPr::new(
                repo_owner.to_string(),
                repo_name.to_string(),
                pr_number,
                None,
                None,
                None,
                None,
            )),
            None,
        )
        .await?;

    let merge_request_issue_id = create_merge_request_issue(
        &env.client,
        patch_id.clone(),
        "requester".to_string(),
        parent_issue_id.clone(),
        "Closed patch".to_string(),
        "Closed description".to_string(),
    )
    .await?
    .id;

    env.run_github_sync(60)
        .await
        .context("sync_open_patches failed")?;

    let updated_patch = env.client.get_patch(&patch_id).await?.patch;
    assert_eq!(updated_patch.status, PatchStatus::Closed);

    let merge_request_issue = env.client.get_issue(&merge_request_issue_id).await?.issue;
    assert_eq!(merge_request_issue.status, IssueStatus::Failed);

    Ok(())
}
