use anyhow::{anyhow, Context, Result};
use metis::command::output::{CommandContext, ResolvedOutputFormat};
use metis::command::patches::create_merge_request_issue;
use metis_common::{
    issues::{IssueStatus, IssueType, JobSettings},
    jobs::SearchJobsQuery,
    patches::{GithubPr, PatchStatus},
};
use metis_server::background::run_spawners::RunSpawnersWorker;
use metis_server::background::scheduler::ScheduledWorker;
use metis_server::background::spawner::AgentQueue;
use metis_server::config::{
    AgentQueueConfig, DEFAULT_AGENT_MAX_SIMULTANEOUS, DEFAULT_AGENT_MAX_TRIES,
};
use metis_server::test_utils::{GitHubMockBuilder, MockPr, MockReview};
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tempfile::TempDir;

mod common;

use common::bash_commands::BashCommands;
use common::test_helpers::init_test_server_with_remote_and_github;

fn git_output(args: &[&str], cwd: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("failed to run git {args:?}"))?;
    if !output.status.success() {
        return Err(anyhow!(
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_output_raw(args: &[&str], cwd: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("failed to run git {args:?}"))?;
    if !output.status.success() {
        return Err(anyhow!(
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn create_branch_with_diff(remote_url: &str, branch: &str, line: &str) -> Result<(String, String)> {
    let tempdir = TempDir::new().context("failed to create tempdir for branch setup")?;
    let repo_path = tempdir.path();

    git_output(&["clone", remote_url, "."], repo_path)?;
    git_output(&["config", "user.name", "Test User"], repo_path)?;
    git_output(&["config", "user.email", "test@example.com"], repo_path)?;
    git_output(&["checkout", "-b", branch], repo_path)?;
    std::fs::OpenOptions::new()
        .append(true)
        .open(repo_path.join("README.md"))
        .context("failed to open README for branch setup")?
        .write_all(line.as_bytes())
        .context("failed to write README content")?;
    git_output(&["add", "README.md"], repo_path)?;
    git_output(&["commit", "-m", "feature change"], repo_path)?;
    git_output(&["push", "-u", "origin", branch], repo_path)?;

    let head_sha = git_output(&["rev-parse", "HEAD"], repo_path)?;
    let diff = git_output_raw(
        &["diff", "--no-ext-diff", "--no-color", "origin/main..HEAD"],
        repo_path,
    )?;

    Ok((head_sha, diff))
}

fn set_remote_head(remote_url: &str, branch: &str) -> Result<()> {
    let output = Command::new("git")
        .args([
            "--git-dir",
            remote_url,
            "symbolic-ref",
            "HEAD",
            &format!("refs/heads/{branch}"),
        ])
        .output()
        .context("failed to update remote HEAD")?;
    if !output.status.success() {
        return Err(anyhow!(
            "git symbolic-ref failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn push_additional_commit(remote_url: &str, branch: &str, line: &str) -> Result<String> {
    let tempdir = TempDir::new().context("failed to create tempdir for extra commit")?;
    let repo_path = tempdir.path();

    git_output(&["clone", remote_url, "."], repo_path)?;
    git_output(&["config", "user.name", "Extra Commit"], repo_path)?;
    git_output(&["config", "user.email", "extra@example.com"], repo_path)?;
    git_output(&["fetch", "origin", branch], repo_path)?;
    git_output(
        &["checkout", "-B", branch, &format!("origin/{branch}")],
        repo_path,
    )?;
    std::fs::OpenOptions::new()
        .append(true)
        .open(repo_path.join("README.md"))
        .context("failed to open README for extra commit")?
        .write_all(line.as_bytes())
        .context("failed to write extra README content")?;
    git_output(&["add", "README.md"], repo_path)?;
    git_output(&["commit", "-m", "additional change"], repo_path)?;
    git_output(&["push", "origin", branch], repo_path)?;

    git_output(&["rev-parse", "HEAD"], repo_path)
}

fn branch_head(remote_url: &str, branch: &str) -> Result<String> {
    let repo = git2::Repository::open(remote_url)
        .with_context(|| format!("failed to open repo at {remote_url}"))?;
    let reference = repo
        .find_reference(&format!("refs/heads/{branch}"))
        .with_context(|| format!("failed to find branch {branch} in remote repo"))?;
    let oid = reference
        .target()
        .ok_or_else(|| anyhow!("branch {branch} has no target"))?;
    Ok(oid.to_string())
}

#[tokio::test]
async fn merge_request_issue_tracks_issue_head_and_merges() -> Result<()> {
    let repo_owner = "octo";
    let repo_name = "repo";
    let pr_number = 99;
    let head_ref = "repo/t-abc123/head";

    let (_github_server, github_app) = GitHubMockBuilder::new()
        .with_installation(repo_owner, repo_name)
        .build()?;

    let mut env = init_test_server_with_remote_and_github("octo/repo", Some(github_app)).await?;
    let repo_config = env
        .state
        .repository_from_store(&env.service_repo_name)
        .await
        .context("failed to load repository config")?;

    let (head_sha, patch_diff) =
        create_branch_with_diff(&repo_config.remote_url, head_ref, "feature change\n")?;
    set_remote_head(&repo_config.remote_url, "main")?;

    // Replace github_app with one whose mocks include the PR
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
            "Code change summary",
            "Code change description",
            patch_diff,
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

    let initial_merge_request_issue_id = create_merge_request_issue(
        &env.client,
        patch_id.clone(),
        "reviewer".to_string(),
        parent_issue_id.clone(),
        "Code change summary".to_string(),
        "Code change description".to_string(),
    )
    .await?
    .id;

    let queue_config = AgentQueueConfig {
        name: "reviewer".to_string(),
        prompt: "Review patch".to_string(),
        max_tries: DEFAULT_AGENT_MAX_TRIES,
        max_simultaneous: DEFAULT_AGENT_MAX_SIMULTANEOUS,
    };
    {
        let mut agents = env.agents.write().await;
        *agents = vec![Arc::new(AgentQueue::from_config(&queue_config))];
    }

    // run the spawner once before syncing and picking up patch changes
    let spawner = RunSpawnersWorker::new(env.state.clone());
    spawner.run_iteration().await;

    env.run_github_sync(60)
        .await
        .context("sync_open_patches failed")?;

    let updated_patch = env.client.get_patch(&patch_id).await?.patch;
    assert_eq!(updated_patch.status, PatchStatus::ChangesRequested);
    assert_eq!(
        updated_patch
            .github
            .as_ref()
            .and_then(|github| github.head_ref.as_deref()),
        Some(head_ref)
    );

    let initial_merge_request_issue = env
        .client
        .get_issue(&initial_merge_request_issue_id)
        .await?
        .issue;
    assert_eq!(initial_merge_request_issue.status, IssueStatus::Failed);

    let merge_request_issue_id = create_merge_request_issue(
        &env.client,
        patch_id.clone(),
        "reviewer".to_string(),
        parent_issue_id.clone(),
        "Code change summary".to_string(),
        "Code change description".to_string(),
    )
    .await?
    .id;

    spawner.run_iteration().await;

    let jobs = env
        .client
        .list_jobs(&SearchJobsQuery::new(
            None,
            Some(merge_request_issue_id.clone()),
            None,
        ))
        .await?
        .jobs;
    let job = jobs
        .first()
        .context("expected review task to be spawned for merge request")?;
    let job_id = job.id.clone();

    env.state.start_pending_task(job_id.clone()).await;

    let main_head_before = branch_head(&repo_config.remote_url, "main")?;
    let worker_dir = tempfile::tempdir().context("failed to create worker tempdir")?;
    let bash_commands = BashCommands::new_with_failure(
        vec![
            "echo \"worker fix\" >> README.md".to_string(),
            "git add README.md".to_string(),
            "git commit -m \"worker fix\"".to_string(),
        ],
        false,
    );
    let context = CommandContext::new(ResolvedOutputFormat::Pretty);
    metis::command::jobs::worker_run::run(
        &env.client,
        job_id,
        worker_dir.path().to_path_buf(),
        None,
        None,
        None,
        Some(merge_request_issue_id.clone()),
        &bash_commands,
        &context,
    )
    .await?;

    let issue_head_branch = format!("metis/{merge_request_issue_id}/head");
    let head_after_worker = branch_head(&repo_config.remote_url, &issue_head_branch)?;
    assert_ne!(head_after_worker, main_head_before);

    let head_after_extra =
        push_additional_commit(&repo_config.remote_url, head_ref, "additional fix\n")?;
    assert_ne!(head_after_extra, head_after_worker);

    // Build a new GitHub mock with the updated head SHA
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
    env.state.github_app = Some(github_app_updated);

    env.run_github_sync(60)
        .await
        .context("sync_open_patches failed after extra commit")?;

    env.run_as_user(vec![
        format!(
            "metis patches review {} --author reviewer --contents \"looks good\" --approve",
            patch_id
        ),
        format!(
            "metis patches merge --repo {} --branch main --patch-id {}",
            env.service_repo_name, patch_id
        ),
    ])
    .await?;

    let merge_queue = env
        .client
        .get_merge_queue(&env.service_repo_name, "main")
        .await?;
    assert!(merge_queue.patches.contains(&patch_id));

    Ok(())
}
