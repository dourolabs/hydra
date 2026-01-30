use anyhow::{anyhow, Context, Result};
use chrono::{TimeZone, Utc};
use httpmock::prelude::*;
use jsonwebtoken::EncodingKey;
use metis::command::patches::create_merge_request_issue;
use metis_common::{
    issues::{IssueStatus, IssueType, JobSettings},
    jobs::SearchJobsQuery,
    patches::{GithubPr, PatchStatus},
};
use metis_server::background::run_spawners::RunSpawnersWorker;
use metis_server::background::scheduler::ScheduledWorker;
use metis_server::background::spawner::{AgentQueue, AGENT_NAME_ENV_VAR, ISSUE_ID_ENV_VAR};
use metis_server::config::{
    AgentQueueConfig, DEFAULT_AGENT_MAX_SIMULTANEOUS, DEFAULT_AGENT_MAX_TRIES,
};
use metis_server::test_utils::github_user_response;
use octocrab::models::AppId;
use octocrab::Octocrab;
use openssl::rsa::Rsa;
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tempfile::TempDir;

mod common;

use common::test_helpers::init_test_server_with_remote_and_github;

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
async fn sync_open_patches_spawns_review_task_for_followup_agent() -> Result<()> {
    let github_server = MockServer::start();
    let github_base_url = github_server.base_url();

    let installation_id = 42;
    let pr_number = 99;
    let repo_owner = "octo";
    let repo_name = "repo";
    let review_branch = "feature/review";

    let private_key = generate_test_rsa_key().context("failed to generate test RSA key")?;
    let github_app = Octocrab::builder()
        .base_uri(github_base_url.clone())
        .context("failed to set mock GitHub base url")?
        .app(
            AppId::from(1),
            EncodingKey::from_rsa_pem(&private_key)
                .context("failed to parse test GitHub App key")?,
        )
        .build()
        .context("failed to build mock GitHub client")?;

    let env = init_test_server_with_remote_and_github("octo/repo", Some(github_app)).await?;
    let repository = env
        .state
        .repository_from_store(&env.service_repo_name)
        .await
        .context("failed to load service repository config")?;
    let head_sha =
        create_branch_with_commit(&repository.remote_url, review_branch, "review change\n")
            .context("failed to create review branch")?;
    let head_sha_for_pr = head_sha.clone();
    let head_sha_for_status = head_sha.clone();
    let head_sha_for_checks = head_sha.clone();

    let installation_mock = github_server.mock(|when, then| {
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

    let token_mock = github_server.mock(|when, then| {
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

    let pr_mock = github_server.mock(|when, then| {
        when.method(GET)
            .path(format!("/repos/{repo_owner}/{repo_name}/pulls/{pr_number}"));
        then.status(200).json_body(json!({
            "url": "",
            "id": 1,
            "number": pr_number,
            "state": "open",
            "locked": false,
            "maintainer_can_modify": false,
            "html_url": "https://example.com/pr/99",
            "merged": false,
            "merged_at": null,
            "head": { "ref": review_branch, "sha": head_sha_for_pr, "user": null, "repo": null },
            "base": { "ref": "main", "sha": "def456", "user": null, "repo": null }
        }));
    });

    let review_time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let reviews_mock = github_server.mock(|when, then| {
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
                "submitted_at": review_time.to_rfc3339(),
                "pull_request_url": "https://example.com/pr/99"
            }
        ]));
    });

    let review_comments_mock = github_server.mock(|when, then| {
        when.method(GET)
            .path(format!(
                "/repos/{repo_owner}/{repo_name}/pulls/{pr_number}/comments"
            ))
            .query_param("per_page", "100");
        then.status(200).json_body(json!([]));
    });

    let issue_comments_mock = github_server.mock(|when, then| {
        when.method(GET)
            .path(format!(
                "/repos/{repo_owner}/{repo_name}/issues/{pr_number}/comments"
            ))
            .query_param("per_page", "100");
        then.status(200).json_body(json!([]));
    });

    let status_mock = github_server.mock(|when, then| {
        when.method(GET).path(format!(
            "/repos/{repo_owner}/{repo_name}/commits/{head_sha_for_status}/status"
        ));
        then.status(200).json_body(json!({
            "state": "success",
            "sha": head_sha_for_status,
            "total_count": 0,
            "statuses": []
        }));
    });

    let checks_mock = github_server.mock(|when, then| {
        when.method(GET)
            .path(format!(
                "/repos/{repo_owner}/{repo_name}/commits/{head_sha_for_checks}/check-runs"
            ))
            .query_param("per_page", "100");
        then.status(200)
            .json_body(json!({ "total_count": 0, "check_runs": [] }));
    });

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

    let followup_agent = env
        .state
        .config
        .background
        .merge_request_followup_agent
        .clone();
    let queue_config = AgentQueueConfig {
        name: followup_agent.clone(),
        prompt: "Review patch".to_string(),
        max_tries: DEFAULT_AGENT_MAX_TRIES,
        max_simultaneous: DEFAULT_AGENT_MAX_SIMULTANEOUS,
    };
    {
        let mut agents = env.agents.write().await;
        *agents = vec![Arc::new(AgentQueue::from_config(&queue_config))];
    }

    env.run_github_sync(60)
        .await
        .context("sync_open_patches failed")?;

    assert!(installation_mock.hits() > 0);
    assert!(token_mock.hits() > 0);
    assert!(pr_mock.hits() > 0);
    assert!(reviews_mock.hits() > 0);
    assert!(review_comments_mock.hits() > 0);
    assert!(issue_comments_mock.hits() > 0);
    assert!(status_mock.hits() > 0);
    assert!(checks_mock.hits() > 0);

    let updated_patch = env.client.get_patch(&patch_id).await?.patch;
    assert_eq!(updated_patch.status, PatchStatus::ChangesRequested);
    assert!(updated_patch
        .reviews
        .iter()
        .any(|review| review.author == "reviewer" && review.contents == "please update"));

    let spawner = RunSpawnersWorker::new(env.state.clone());
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

    env.state
        .transition_task_to_running(&job.id)
        .await
        .context("failed to mark review job as running")?;

    let outputs = env
        .run_as_worker(vec!["git rev-parse HEAD".to_string()], job.id.clone())
        .await
        .context("failed to run worker job for review branch")?;
    let head_output = outputs
        .last()
        .context("expected worker output for HEAD rev-parse")?;
    assert_eq!(head_output.stdout.trim(), head_sha);

    assert_eq!(job.task.spawned_from, Some(merge_request_issue_id.clone()));
    assert_eq!(
        job.task
            .env_vars
            .get(AGENT_NAME_ENV_VAR)
            .map(String::as_str),
        Some(followup_agent.as_str())
    );
    assert_eq!(
        job.task.env_vars.get(ISSUE_ID_ENV_VAR).map(String::as_str),
        Some(merge_request_issue_id.as_ref())
    );

    Ok(())
}
