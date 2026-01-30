use anyhow::{anyhow, Context, Result};
use httpmock::prelude::*;
use httpmock::Mock;
use jsonwebtoken::EncodingKey;
use metis::command::jobs::worker_run::resolve_tracking_branch_override;
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
use metis_server::test_utils::github_user_response;
use octocrab::models::AppId;
use octocrab::Octocrab;
use openssl::rsa::Rsa;
use serde_json::json;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tempfile::TempDir;

mod common;

use common::bash_commands::BashCommands;
use common::test_helpers::init_test_server_with_remote_and_github;

struct GithubMocks<'a> {
    installation: Mock<'a>,
    token: Mock<'a>,
    pr: Mock<'a>,
    reviews: Mock<'a>,
    review_comments: Mock<'a>,
    issue_comments: Mock<'a>,
    status: Mock<'a>,
    checks: Mock<'a>,
}

fn generate_test_rsa_key() -> Result<Vec<u8>> {
    Ok(Rsa::generate(2048)?.private_key_to_pem()?)
}

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

fn setup_github_mocks<'a>(
    github_server: &'a MockServer,
    repo_owner: &str,
    repo_name: &str,
    pr_number: u64,
    head_sha: &str,
    head_ref: &str,
) -> GithubMocks<'a> {
    let github_base_url = github_server.base_url();
    let installation_id = 42;

    let installation = github_server.mock(|when, then| {
        when.method(GET)
            .path(format!("/repos/{repo_owner}/{repo_name}/installation"));
        then.status(200).json_body(json!({
            "id": installation_id,
            "app_id": 1,
            "account": github_user_response(repo_owner, 1),
            "repository_selection": "selected",
            "access_tokens_url": format!(
                "{}/app/installations/{}/access_tokens",
                github_base_url, installation_id
            ),
            "repositories_url": format!("{}/installation/repositories", github_base_url),
            "html_url": "https://github.com/apps/test/installations/1",
            "app_slug": "test-app",
            "target_id": 1,
            "target_type": "Organization",
            "permissions": {},
            "events": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z"
        }));
    });

    let token = github_server.mock(|when, then| {
        when.method(POST).path(format!(
            "/app/installations/{installation_id}/access_tokens"
        ));
        then.status(201).json_body(json!({
            "token": "gh-install-token",
            "expires_at": "2030-01-01T00:00:00Z",
            "permissions": {},
            "repositories": []
        }));
    });

    let pr = github_server.mock(|when, then| {
        when.method(GET)
            .path(format!("/repos/{repo_owner}/{repo_name}/pulls/{pr_number}"));
        then.status(200).json_body(json!({
            "url": "",
            "id": 1,
            "number": pr_number,
            "state": "open",
            "locked": false,
            "maintainer_can_modify": false,
            "html_url": format!("https://example.com/pr/{pr_number}"),
            "merged": false,
            "merged_at": null,
            "head": { "ref": head_ref, "sha": head_sha, "user": null, "repo": null },
            "base": { "ref": "main", "sha": "def456", "user": null, "repo": null }
        }));
    });

    let reviews = github_server.mock(|when, then| {
        when.method(GET)
            .path(format!(
                "/repos/{repo_owner}/{repo_name}/pulls/{pr_number}/reviews"
            ))
            .query_param("per_page", "100");
        then.status(200).json_body(json!([
            {
                "id": 101,
                "node_id": "NODEID",
                "html_url": "https://example.com/reviews/101",
                "body": "please update",
                "state": "CHANGES_REQUESTED",
                "user": github_user_response("reviewer", 1001),
                "submitted_at": "2024-01-01T00:00:00Z",
                "pull_request_url": format!("https://example.com/pr/{pr_number}")
            }
        ]));
    });

    let review_comments = github_server.mock(|when, then| {
        when.method(GET)
            .path(format!(
                "/repos/{repo_owner}/{repo_name}/pulls/{pr_number}/comments"
            ))
            .query_param("per_page", "100");
        then.status(200).json_body(json!([]));
    });

    let issue_comments = github_server.mock(|when, then| {
        when.method(GET)
            .path(format!(
                "/repos/{repo_owner}/{repo_name}/issues/{pr_number}/comments"
            ))
            .query_param("per_page", "100");
        then.status(200).json_body(json!([]));
    });

    let status = github_server.mock(|when, then| {
        when.method(GET).path(format!(
            "/repos/{repo_owner}/{repo_name}/commits/{head_sha}/status"
        ));
        then.status(200).json_body(json!({
            "state": "success",
            "sha": head_sha,
            "total_count": 0,
            "statuses": []
        }));
    });

    let checks = github_server.mock(|when, then| {
        when.method(GET)
            .path(format!(
                "/repos/{repo_owner}/{repo_name}/commits/{head_sha}/check-runs"
            ))
            .query_param("per_page", "100");
        then.status(200)
            .json_body(json!({ "total_count": 0, "check_runs": [] }));
    });

    GithubMocks {
        installation,
        token,
        pr,
        reviews,
        review_comments,
        issue_comments,
        status,
        checks,
    }
}

#[tokio::test]
async fn merge_request_override_accepts_additional_commits_and_merges() -> Result<()> {
    let github_server = MockServer::start();
    let repo_owner = "octo";
    let repo_name = "repo";
    let pr_number = 99;
    let head_ref = "repo/t-abc123/head";

    let private_key = generate_test_rsa_key().context("failed to generate test RSA key")?;
    let github_app = Octocrab::builder()
        .base_uri(github_server.base_url())
        .context("failed to set mock GitHub base url")?
        .app(
            AppId::from(1),
            EncodingKey::from_rsa_pem(&private_key)
                .context("failed to parse test GitHub App key")?,
        )
        .build()
        .context("failed to build mock GitHub client")?;

    let mut env = init_test_server_with_remote_and_github("octo/repo", Some(github_app)).await?;
    let repo_config = env
        .state
        .repository_from_store(&env.service_repo_name)
        .await
        .context("failed to load repository config")?;

    let (head_sha, patch_diff) =
        create_branch_with_diff(&repo_config.remote_url, head_ref, "feature change\n")?;
    set_remote_head(&repo_config.remote_url, "main")?;

    let mocks = setup_github_mocks(
        &github_server,
        repo_owner,
        repo_name,
        pr_number,
        &head_sha,
        head_ref,
    );

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

    let merge_request_issue_id = create_merge_request_issue(
        &env.client,
        patch_id.clone(),
        "requester".to_string(),
        parent_issue_id.clone(),
        "Code change summary".to_string(),
        "Code change description".to_string(),
    )
    .await?
    .id;

    let queue_config = AgentQueueConfig {
        name: "swe".to_string(),
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

    assert!(mocks.installation.hits() > 0);
    assert!(mocks.token.hits() > 0);
    assert!(mocks.pr.hits() > 0);
    assert!(mocks.reviews.hits() > 0);
    assert!(mocks.review_comments.hits() > 0);
    assert!(mocks.issue_comments.hits() > 0);
    assert!(mocks.status.hits() > 0);
    assert!(mocks.checks.hits() > 0);

    let updated_patch = env.client.get_patch(&patch_id).await?.patch;
    assert_eq!(updated_patch.status, PatchStatus::ChangesRequested);
    assert_eq!(
        updated_patch
            .github
            .as_ref()
            .and_then(|github| github.head_ref.as_deref()),
        Some(head_ref)
    );

    spawner.run_iteration().await;

    let jobs = env
        .client
        .list_jobs(&SearchJobsQuery::new(
            None,
            Some(merge_request_issue_id.clone()),
        ))
        .await?
        .jobs;
    let job = jobs
        .first()
        .context("expected review task to be spawned for merge request")?;
    let job_id = job.id.clone();

    env.state.start_pending_task(job_id.clone()).await;

    let override_branch =
        resolve_tracking_branch_override(&env.client, &merge_request_issue_id).await?;
    assert_eq!(override_branch.as_deref(), Some(head_ref));

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

    let head_after_worker = branch_head(&repo_config.remote_url, head_ref)?;
    assert_ne!(head_after_worker, head_sha);

    let head_after_extra =
        push_additional_commit(&repo_config.remote_url, head_ref, "additional fix\n")?;
    assert_ne!(head_after_extra, head_after_worker);

    let github_server_updated = MockServer::start();
    let github_app_updated = Octocrab::builder()
        .base_uri(github_server_updated.base_url())
        .context("failed to set mock GitHub base url")?
        .app(
            AppId::from(1),
            EncodingKey::from_rsa_pem(&private_key)
                .context("failed to parse test GitHub App key")?,
        )
        .build()
        .context("failed to build updated mock GitHub client")?;
    env.state.github_app = Some(github_app_updated);

    let updated_mocks = setup_github_mocks(
        &github_server_updated,
        repo_owner,
        repo_name,
        pr_number,
        &head_after_extra,
        head_ref,
    );

    env.run_github_sync(60)
        .await
        .context("sync_open_patches failed after extra commit")?;

    assert!(updated_mocks.status.hits() > 0);
    assert!(updated_mocks.checks.hits() > 0);

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
