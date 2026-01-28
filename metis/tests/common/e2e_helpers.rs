#![allow(dead_code)]

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use httpmock::{Mock, MockServer};
use metis::client::MetisClient;
use metis::config::{AppConfig, ServerSection};
use metis_common::issues::{Issue, IssueStatus, IssueType, JobSettings, UpsertIssueRequest};
use metis_common::patches::{
    GithubPr, Patch, PatchStatus, UpsertPatchRequest, UpsertPatchResponse,
};
use metis_common::users::Username;
use metis_common::{IssueId, PatchId, RepoName, TaskId};
use metis_server::app::{AppState, ServiceState};
use metis_server::background::poll_github_patches::GithubPollerWorker;
use metis_server::background::scheduler::{ScheduledWorker, WorkerOutcome};
use metis_server::store::{MemoryStore, Store};
use metis_server::test_utils::{spawn_test_server_with_state, test_app_config, MockJobEngine};
use octocrab::Octocrab;
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::sync::RwLock;

pub struct E2eHarness {
    pub server: metis_server::test_utils::TestServer,
    pub app_config: AppConfig,
    pub client: MetisClient,
    pub state: AppState,
    pub store: Arc<dyn Store>,
    pub _tempdir: TempDir,
    pub service_repo_name: RepoName,
    pub auth_token: String,
    pub current_issue_id: IssueId,
}

pub struct GithubFixture {
    pub server: MockServer,
    pub owner: String,
    pub repo: String,
}

impl GithubFixture {
    pub async fn new(owner: impl Into<String>, repo: impl Into<String>) -> Result<Self> {
        let server = MockServer::start_async().await;
        Ok(Self {
            server,
            owner: owner.into(),
            repo: repo.into(),
        })
    }

    pub fn api_base_url(&self) -> String {
        self.server.base_url()
    }

    pub fn github_pr(&self, number: u64) -> GithubPr {
        GithubPr::new(
            self.owner.clone(),
            self.repo.clone(),
            number,
            None,
            None,
            None,
            None,
        )
    }

    pub fn mock_installation(&self, installation_id: u64) -> Mock {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        self.server.mock(move |when, then| {
            when.method(httpmock::Method::GET)
                .path(format!("/repos/{owner}/{repo}/installation"));
            then.status(200).json_body(json!({
                "id": installation_id,
                "account": github_user_response(&owner, 1),
                "repository_selection": "all",
                "access_tokens_url": format!("https://api.github.com/app/installations/{installation_id}/access_tokens"),
                "repositories_url": "https://api.github.com/installation/repositories",
                "html_url": "https://github.com/apps/metis",
                "app_id": 1,
                "target_id": 1,
                "target_type": "Organization",
                "permissions": {},
                "events": [],
                "created_at": "2023-01-01T00:00:00Z",
                "updated_at": "2023-01-01T00:00:00Z"
            }));
        })
    }

    pub fn mock_installation_token(&self, installation_id: u64, token: &str) -> Mock {
        let token = token.to_string();
        self.server.mock(move |when, then| {
            when.method(httpmock::Method::POST).path(format!(
                "/app/installations/{installation_id}/access_tokens"
            ));
            then.status(200).json_body(json!({
                "token": token,
                "expires_at": "2030-01-01T00:00:00Z",
                "permissions": {},
                "repository_selection": "all"
            }));
        })
    }

    pub fn mock_pull_request(
        &self,
        number: u64,
        state: &str,
        merged: bool,
        head_sha: &str,
        head_ref: &str,
        base_ref: &str,
    ) -> Mock {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let head_sha = head_sha.to_string();
        let head_ref = head_ref.to_string();
        let base_ref = base_ref.to_string();
        let merged_at = if merged {
            Some("2023-01-01T00:00:00Z")
        } else {
            None
        };
        self.server.mock(move |when, then| {
            when.method(httpmock::Method::GET)
                .path(format!("/repos/{owner}/{repo}/pulls/{number}"));
            then.status(200).json_body(json!({
                "id": 1,
                "number": number,
                "state": state,
                "merged": merged,
                "merged_at": merged_at,
                "html_url": format!("https://github.com/{owner}/{repo}/pull/{number}"),
                "head": {
                    "ref": head_ref,
                    "sha": head_sha,
                    "repo": {
                        "name": repo,
                        "full_name": format!("{owner}/{repo}"),
                        "owner": github_user_response(&owner, 1)
                    }
                },
                "base": {
                    "ref": base_ref,
                    "sha": "base-sha",
                    "repo": {
                        "name": repo,
                        "full_name": format!("{owner}/{repo}"),
                        "owner": github_user_response(&owner, 1)
                    }
                },
                "user": github_user_response(&owner, 1),
                "draft": false
            }));
        })
    }

    pub fn mock_reviews(&self, number: u64, reviews: Vec<serde_json::Value>) -> Mock {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        self.server.mock(move |when, then| {
            when.method(httpmock::Method::GET)
                .path(format!("/repos/{owner}/{repo}/pulls/{number}/reviews"));
            then.status(200).json_body(reviews);
        })
    }

    pub fn mock_review_comments(&self, number: u64, comments: Vec<serde_json::Value>) -> Mock {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        self.server.mock(move |when, then| {
            when.method(httpmock::Method::GET)
                .path(format!("/repos/{owner}/{repo}/pulls/{number}/comments"));
            then.status(200).json_body(comments);
        })
    }

    pub fn mock_issue_comments(&self, number: u64, comments: Vec<serde_json::Value>) -> Mock {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        self.server.mock(move |when, then| {
            when.method(httpmock::Method::GET)
                .path(format!("/repos/{owner}/{repo}/issues/{number}/comments"));
            then.status(200).json_body(comments);
        })
    }

    pub fn mock_combined_status(
        &self,
        sha: &str,
        state: &str,
        statuses: Vec<serde_json::Value>,
    ) -> Mock {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let sha = sha.to_string();
        self.server.mock(move |when, then| {
            when.method(httpmock::Method::GET)
                .path(format!("/repos/{owner}/{repo}/commits/{sha}/status"));
            then.status(200).json_body(json!({
                "state": state,
                "sha": sha,
                "total_count": statuses.len(),
                "statuses": statuses,
                "repository": null,
                "commit_url": null,
                "url": null
            }));
        })
    }

    pub fn mock_check_runs(&self, sha: &str, check_runs: Vec<serde_json::Value>) -> Mock {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let sha = sha.to_string();
        self.server.mock(move |when, then| {
            when.method(httpmock::Method::GET)
                .path(format!("/repos/{owner}/{repo}/commits/{sha}/check-runs"));
            then.status(200).json_body(json!({
                "total_count": check_runs.len(),
                "check_runs": check_runs
            }));
        })
    }
}

pub async fn init_e2e_harness(repo_name: &str) -> Result<E2eHarness> {
    init_e2e_harness_with_github(repo_name, None).await
}

pub async fn init_e2e_harness_with_github(
    repo_name: &str,
    github: Option<&GithubFixture>,
) -> Result<E2eHarness> {
    let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
    let remote_url = init_service_remote(tempdir.path())?;
    let service_repo_name = RepoName::from_str(repo_name)
        .with_context(|| format!("failed to parse service repo name: {repo_name}"))?;
    let github_api_base = github.map(|fixture| fixture.api_base_url());
    let (state, store, auth_token) =
        app_state_with_repo(&remote_url, &service_repo_name, github_api_base).await?;
    let server = spawn_test_server_with_state(state.clone(), store.clone())
        .await
        .context("failed to start test server")?;
    let server_url = server.base_url();

    let app_config = AppConfig {
        server: ServerSection { url: server_url },
    };
    let client = MetisClient::from_config(&app_config, auth_token.clone())?;
    let current_issue_id = create_task_issue(
        &client,
        "current issue context",
        Username::from("test-user"),
        default_job_settings(service_repo_name.clone()),
        Vec::new(),
    )
    .await?;

    Ok(E2eHarness {
        server,
        app_config,
        client,
        state,
        store,
        _tempdir: tempdir,
        service_repo_name,
        auth_token,
        current_issue_id,
    })
}

pub fn default_job_settings(repo_name: RepoName) -> JobSettings {
    let mut settings = JobSettings::default();
    settings.repo_name = Some(repo_name);
    settings.image = Some("worker:latest".into());
    settings.branch = Some("main".into());
    settings
}

pub async fn create_task_issue(
    client: &MetisClient,
    description: &str,
    creator: Username,
    job_settings: JobSettings,
    dependencies: Vec<metis_common::issues::IssueDependency>,
) -> Result<IssueId> {
    let issue = Issue::new(
        IssueType::Task,
        description.to_string(),
        creator,
        String::new(),
        IssueStatus::Open,
        None,
        Some(job_settings),
        Vec::new(),
        dependencies,
        Vec::new(),
    );
    let response = client
        .create_issue(&UpsertIssueRequest::new(issue, None))
        .await?;
    Ok(response.issue_id)
}

pub async fn create_patch(client: &MetisClient, patch: Patch) -> Result<PatchId> {
    let response: UpsertPatchResponse =
        client.create_patch(&UpsertPatchRequest::new(patch)).await?;
    Ok(response.patch_id)
}

pub fn default_patch(service_repo_name: RepoName, github: Option<GithubPr>) -> Patch {
    Patch::new(
        "test patch".to_string(),
        "test patch description".to_string(),
        sample_diff(),
        PatchStatus::Open,
        false,
        None,
        Vec::new(),
        service_repo_name,
        github,
    )
}

pub fn sample_diff() -> String {
    "--- a/README.md\n+++ b/README.md\n@@\n-old\n+new\n".to_string()
}

pub fn github_review(
    body: &str,
    author: &str,
    state: &str,
    submitted_at: DateTime<Utc>,
) -> serde_json::Value {
    json!({
        "id": 1,
        "body": body,
        "state": state,
        "user": github_user_response(author, 1),
        "submitted_at": submitted_at.to_rfc3339()
    })
}

pub fn github_review_comment(
    body: &str,
    author: &str,
    created_at: DateTime<Utc>,
) -> serde_json::Value {
    json!({
        "id": 1,
        "body": body,
        "user": github_user_response(author, 1),
        "created_at": created_at.to_rfc3339()
    })
}

pub fn github_issue_comment(
    body: &str,
    author: &str,
    created_at: DateTime<Utc>,
) -> serde_json::Value {
    json!({
        "id": 1,
        "body": body,
        "user": github_user_response(author, 1),
        "created_at": created_at.to_rfc3339()
    })
}

pub fn github_status(
    state: &str,
    context: &str,
    description: Option<&str>,
    target_url: Option<&str>,
) -> serde_json::Value {
    json!({
        "id": null,
        "node_id": null,
        "avatar_url": null,
        "description": description,
        "url": null,
        "target_url": target_url,
        "created_at": null,
        "updated_at": null,
        "state": state,
        "creator": null,
        "context": context
    })
}

pub fn github_check_run(
    name: &str,
    conclusion: Option<&str>,
    summary: Option<&str>,
    details_url: Option<&str>,
) -> serde_json::Value {
    json!({
        "id": 1,
        "node_id": null,
        "head_sha": "abc123",
        "url": format!("https://api.example.com/checks/{name}"),
        "html_url": details_url,
        "details_url": details_url,
        "conclusion": conclusion,
        "output": {
            "title": name,
            "summary": summary,
            "text": null,
            "annotations_count": 0,
            "annotations_url": "https://ci.example.com/annotations"
        },
        "started_at": null,
        "completed_at": null,
        "name": name,
        "pull_requests": []
    })
}

pub async fn trigger_github_sync(state: &AppState) -> Result<WorkerOutcome> {
    let worker = GithubPollerWorker::new(state.clone(), 1);
    Ok(worker.run_iteration().await)
}

pub async fn wait_for_patch(
    client: &MetisClient,
    patch_id: &PatchId,
    mut predicate: impl FnMut(&Patch) -> bool,
) -> Result<Patch> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if Instant::now() > deadline {
            bail!("timed out waiting for patch '{patch_id}'");
        }

        let patch = client.get_patch(patch_id).await?.patch;
        if predicate(&patch) {
            return Ok(patch);
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn init_service_remote(base_dir: &Path) -> Result<String> {
    let workdir = base_dir.join("workdir");
    let remote_dir = base_dir.join("remote.git");
    let workdir_str = workdir
        .to_str()
        .ok_or_else(|| anyhow!("workdir path contains invalid UTF-8"))?;
    let remote_dir_str = remote_dir
        .to_str()
        .ok_or_else(|| anyhow!("remote dir path contains invalid UTF-8"))?;

    Command::new("git")
        .args(["init", workdir_str])
        .status()
        .context("failed to init workdir")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git init returned non-zero exit code"))?;
    Command::new("git")
        .args(["-C", workdir_str, "checkout", "-b", "main"])
        .status()
        .context("failed to create main branch in workdir")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git checkout returned non-zero exit code"))?;
    Command::new("git")
        .args([
            "-C",
            workdir_str,
            "config",
            "user.name",
            "Worker Integration",
        ])
        .status()
        .context("failed to set git user.name")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git config user.name returned non-zero exit code"))?;
    Command::new("git")
        .args([
            "-C",
            workdir_str,
            "config",
            "user.email",
            "worker@example.com",
        ])
        .status()
        .context("failed to set git user.email")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git config user.email returned non-zero exit code"))?;
    std::fs::write(workdir.join("README.md"), "base content\n")
        .context("failed to write initial README")?;
    Command::new("git")
        .args(["-C", workdir_str, "add", "README.md"])
        .status()
        .context("failed to add README.md")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git add returned non-zero exit code"))?;
    Command::new("git")
        .args(["-C", workdir_str, "commit", "-m", "initial commit"])
        .status()
        .context("failed to commit README")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git commit returned non-zero exit code"))?;

    Command::new("git")
        .args(["init", "--bare", remote_dir_str])
        .status()
        .context("failed to init bare remote")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git init --bare returned non-zero exit code"))?;
    Command::new("git")
        .args(["-C", workdir_str, "remote", "add", "origin", remote_dir_str])
        .status()
        .context("failed to add remote")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git remote add returned non-zero exit code"))?;
    Command::new("git")
        .args(["-C", workdir_str, "push", "-u", "origin", "main"])
        .status()
        .context("failed to push initial commit to remote")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git push returned non-zero exit code"))?;
    Command::new("git")
        .args([
            "--git-dir",
            remote_dir_str,
            "symbolic-ref",
            "HEAD",
            "refs/heads/main",
        ])
        .status()
        .context("failed to set remote HEAD")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git symbolic-ref returned non-zero exit code"))?;

    Ok(remote_dir_str.to_string())
}

async fn app_state_with_repo(
    remote_url: &str,
    repo_name: &RepoName,
    github_api_base_url: Option<String>,
) -> Result<(AppState, Arc<dyn Store>, String)> {
    let mut server_config = test_app_config();
    let github_app = if let Some(api_base) = github_api_base_url {
        server_config.github_app.api_base_url = api_base.clone();
        server_config.github_app.oauth_base_url = api_base;
        Some(
            Octocrab::builder()
                .base_uri(server_config.github_app.api_base_url().to_string())
                .map_err(|err| anyhow!("failed to parse github api base url: {err}"))?
                .personal_token("gh-app-test-token".to_string())
                .build()
                .map_err(|err| anyhow!("failed to build github app client: {err}"))?,
        )
    } else {
        server_config.github_app.api_base_url = String::new();
        server_config.github_app.oauth_base_url = String::new();
        None
    };

    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    store
        .add_repository(
            repo_name.clone(),
            metis_common::repositories::Repository::new(
                remote_url.to_string(),
                Some("main".to_string()),
                None,
            ),
        )
        .await?;

    let (actor, auth_token) = metis_server::domain::actors::Actor::new_for_task(TaskId::new());
    store.add_actor(actor).await?;
    let user = metis_common::users::User::new(
        Username::from("test-user"),
        1,
        auth_token.clone(),
        "gh-refresh-token".to_string(),
    );
    store.add_user(user.into()).await?;

    Ok((
        AppState::new(
            Arc::new(server_config),
            github_app,
            Arc::new(ServiceState::default()),
            store.clone(),
            Arc::new(MockJobEngine::new()),
            Arc::new(RwLock::new(Vec::new())),
        ),
        store,
        auth_token,
    ))
}

fn github_user_response(login: &str, id: u64) -> serde_json::Value {
    json!({
        "login": login,
        "id": id,
        "node_id": "NODEID",
        "avatar_url": "https://example.com/avatar",
        "gravatar_id": "gravatar",
        "url": "https://example.com/user",
        "html_url": "https://example.com/user",
        "followers_url": "https://example.com/followers",
        "following_url": "https://example.com/following",
        "gists_url": "https://example.com/gists",
        "starred_url": "https://example.com/starred",
        "subscriptions_url": "https://example.com/subscriptions",
        "organizations_url": "https://example.com/orgs",
        "repos_url": "https://example.com/repos",
        "events_url": "https://example.com/events",
        "received_events_url": "https://example.com/received_events",
        "type": "User",
        "site_admin": false,
        "name": null,
        "patch_url": null,
        "email": null
    })
}
