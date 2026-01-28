use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use httpmock::prelude::*;
use jsonwebtoken::EncodingKey;
use metis_common::{
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType, JobSettings,
        UpsertIssueRequest,
    },
    jobs::SearchJobsQuery,
    patches::{GithubPr, PatchStatus},
    users::Username,
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
use serde_json::json;
use std::sync::Arc;

mod common;

use common::test_helpers::init_test_server_with_remote_and_github;

const TEST_APP_PRIVATE_KEY: &str = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpQIBAAKCAQEAzOglZTqWKm1YRYk/bxQvHDpIZw1DGdXu8xGDfk54GIkdT+Gq\neg+unyByTaoEHKr9CrUx2zbpLRdjJpt2paNmMAd9nXEl/mQvwiRSiKhSJfqlbLPP\nupBCHYlQP+PpCEJgsm9Hj3FzqkBpz+sC/ZxXJdzYhmEIISKjj5a64eoMdM3UOyZ4\nAQTPkkfGlFVnifnj72dJrLH8doT6obJ3GgulDWDD25ci4ZYaaQivNcVJQvfpdzm2\nzYydmgr5OSnRfu7iDQWt5+kka4h7t1uH9q9GE9vqlb3N/1gYr54c2HmRrd9VxnJH\nSpZXl03bucDS/CwEvBZHIbbmrDoZpNbRBZ1ylwIDAQABAoIBABYTZK6qndxlpL7z\nxxvGhuokSqyfkdJ0vqZSsAypk5LBIvak5LhQfDW0nzFILI08zBTylIko5Kebqhj8\nuCRRna7K/844jylVzeISsQ78DhhSuq54E4c8B4N7Hw7jFQtza0uOEEhJ/CDO3mzX\ncEkL0O/JV/fnovfpmB7eKbWGgQpho43WSfKBPpp8pX/As2VCu9k3Sjk3WcfeOMIT\nHLN7KufkJeEQlxuX4mU+eURyqIoylUwZXYDilWWvSJuIAMmnUo66u4cvG/PBMKIN\neBxxrj1eFiOyA1/UyclztEBoZEoHqG0KbeisbjjkCBQTmn7ywWEixIwCjU1fNpfT\nilXWfLECgYEA/3ynOByvF1XpyofjxOWr14JRfw3fCWecI05s6POo+kP6a3G2GRO7\nMX1bHwpKefYl+0jzjLVToWDVCtQ0N+RAnnbiZRPiZ3TN7hPufMogCgytcu1WRClT\nS5BMCWzKLpI69tsRITypPJXtRiVwRJPo4fCGrFlKzPDPw58OF2VBiL0CgYEAzVF9\nTKyNXtwFzryfWatoFt645Y2hHr1//JxxhOtKe0y7xpE8w0PfYckOngLHFVFlZQgC\nw75ZlRL4jJ2L+7qI4yP0EXqFPAzaQI8R4xnVjHT03EqjRpKM4FR3y0n3fVGYPo19\nnMTRwl56AG//GhL9AKtDNcpdTu+t+BQpSMSor+MCgYEA6uFT7p9YTVDb1inmOc+Y\nk1Go0PEUutW5UzA3qlbQY/z5DayF6Doen9oKWtggLk4hDws7dYICt9uJISKEO1oq\nGkVbz+de/xQAer9yQuGkYPjUwVL3O0Tu4gpwDT4qBnTDps0xy2e0gxGnCRVESJfe\nw1FYzrxsq0s9BzCESPf7LtUCgYEAobCqB2bgEjMdk7ixmTFGULRnUcfeedHsZ+hf\n8bhGOKGuQur/uhrKYTyv+TngxGYMfqr3WmWeMKr29+3eXoiA4rferqEZKbhJbIv/\nHySqKum0J4PT33Dr5oI+sOZ4M8W9Ko3MvVe2hOZYF94bPNJ1UkCNNmA+aTqRe4uN\nE5Rj79cCgYEA3as0X+av2mIyGkRMswBZAG46LYV7TEp+lK8TAsTZ4+jHlcbk5EZc\nNFmgjtmrgZ5aOpOQdtztLXJ8JHBNTqXuw0jDmgSXhYM0GiyZNZcctX0ADspAqAMv\nlJTCale/1jva/ErqSrdOJgGi6xeypuvr121MV5eiHvRHZEN+Pg/NW2M=\n-----END RSA PRIVATE KEY-----\n";

#[tokio::test]
async fn sync_open_patches_spawns_review_task_for_swe() -> Result<()> {
    let github_server = MockServer::start();
    let github_base_url = github_server.base_url();

    let installation_id = 42;
    let pr_number = 99;
    let repo_owner = "octo";
    let repo_name = "repo";
    let head_sha = "abc123";

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
            "head": { "ref": "feature/review", "sha": head_sha, "user": null, "repo": null },
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
            "/repos/{repo_owner}/{repo_name}/commits/{head_sha}/status"
        ));
        then.status(200).json_body(json!({
            "state": "success",
            "sha": head_sha,
            "total_count": 0,
            "statuses": []
        }));
    });

    let checks_mock = github_server.mock(|when, then| {
        when.method(GET)
            .path(format!(
                "/repos/{repo_owner}/{repo_name}/commits/{head_sha}/check-runs"
            ))
            .query_param("per_page", "100");
        then.status(200)
            .json_body(json!({ "total_count": 0, "check_runs": [] }));
    });

    let github_app = Octocrab::builder()
        .base_uri(github_base_url)
        .context("failed to set mock GitHub base url")?
        .app(
            AppId::from(1),
            EncodingKey::from_rsa_pem(TEST_APP_PRIVATE_KEY.as_bytes())
                .context("failed to parse test GitHub App key")?,
        )
        .build()
        .context("failed to build mock GitHub client")?;

    let env = init_test_server_with_remote_and_github("octo/repo", Some(github_app)).await?;

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

    let merge_request_issue = Issue::new(
        IssueType::MergeRequest,
        format!("Review patch {}", patch_id.as_ref()),
        Username::from("requester"),
        String::new(),
        IssueStatus::Open,
        Some("requester".to_string()),
        Some(job_settings),
        Vec::new(),
        vec![IssueDependency::new(
            IssueDependencyType::ChildOf,
            parent_issue_id.clone(),
        )],
        vec![patch_id.clone()],
    );

    let merge_request_issue_id = env
        .client
        .create_issue(&UpsertIssueRequest::new(merge_request_issue, None))
        .await?
        .issue_id;

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

    assert_eq!(job.task.spawned_from, Some(merge_request_issue_id.clone()));
    assert_eq!(
        job.task
            .env_vars
            .get(AGENT_NAME_ENV_VAR)
            .map(String::as_str),
        Some("swe")
    );
    assert_eq!(
        job.task.env_vars.get(ISSUE_ID_ENV_VAR).map(String::as_str),
        Some(merge_request_issue_id.as_ref())
    );

    Ok(())
}
