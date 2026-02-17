#![allow(dead_code)]

use anyhow::{Context, Result};
use metis::client::MetisClient;
use metis::config::{AppConfig, ServerSection};
use metis_common::{
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType, IssueVersionRecord,
        JobSettings, ListIssuesResponse, SearchIssuesQuery, UpsertIssueRequest,
    },
    jobs::{BundleSpec, CreateJobRequest, SearchJobsQuery},
    patches::{
        GithubPr, ListPatchesResponse, Patch, PatchStatus, PatchVersionRecord, SearchPatchesQuery,
        UpsertPatchRequest,
    },
    users::Username,
    IssueId, PatchId, RepoName, TaskId,
};
use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Command;

/// Output captured from a CLI subprocess invocation.
#[derive(Debug, Clone)]
pub struct CliOutput {
    pub stdout: String,
    pub stderr: String,
    pub status: std::process::ExitStatus,
}

/// A typed actor handle that wraps `MetisClient` with pre-filled authentication.
///
/// Tests interact with the system through `UserHandle` instances obtained from
/// the `TestHarness`. Each handle represents a named actor (user or agent) with
/// a pre-configured auth token. The typed API methods construct the appropriate
/// request types internally, so tests can express intent concisely:
///
/// ```ignore
/// let user = harness.default_user();
/// let issue_id = user.create_issue("fix the bug").await?;
/// let issue = user.get_issue(&issue_id).await?;
/// ```
pub struct UserHandle {
    name: String,
    token: String,
    client: MetisClient,
    server_url: String,
}

impl UserHandle {
    /// Create a new `UserHandle` for the given user name, token, and server URL.
    pub(crate) fn new(name: String, token: String, server_url: &str) -> Result<Self> {
        let config = AppConfig {
            servers: vec![ServerSection {
                url: server_url.to_string(),
                auth_token: None,
                default: true,
            }],
        };
        let client = MetisClient::from_config(&config, &token)
            .context("failed to create MetisClient for UserHandle")?;
        Ok(Self {
            name,
            token,
            client,
            server_url: server_url.to_string(),
        })
    }

    /// Return the user's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the user's auth token.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Return a reference to the underlying `MetisClient`.
    pub fn client(&self) -> &MetisClient {
        &self.client
    }

    // ── Issue operations ─────────────────────────────────────────────

    /// Create a new issue with the given description. Returns the new issue's ID.
    pub async fn create_issue(&self, description: &str) -> Result<IssueId> {
        let issue = Issue::new(
            IssueType::Task,
            description.to_string(),
            Username::from(self.name.as_str()),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            false,
        );
        let request = UpsertIssueRequest::new(issue, None);
        let response = self
            .client
            .create_issue(&request)
            .await
            .context("UserHandle::create_issue failed")?;
        Ok(response.issue_id)
    }

    /// Create a child issue under the given parent. Returns the new issue's ID.
    pub async fn create_child_issue(&self, parent: &IssueId, description: &str) -> Result<IssueId> {
        let issue = Issue::new(
            IssueType::Task,
            description.to_string(),
            Username::from(self.name.as_str()),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent.clone(),
            )],
            Vec::new(),
            false,
        );
        let request = UpsertIssueRequest::new(issue, None);
        let response = self
            .client
            .create_issue(&request)
            .await
            .context("UserHandle::create_child_issue failed")?;
        Ok(response.issue_id)
    }

    /// Update the status of an existing issue.
    pub async fn update_issue_status(&self, id: &IssueId, status: IssueStatus) -> Result<()> {
        let existing = self
            .client
            .get_issue(id, false)
            .await
            .context("UserHandle::update_issue_status: failed to get issue")?;
        let mut issue = existing.issue;
        issue.status = status;
        let request = UpsertIssueRequest::new(issue, None);
        self.client
            .update_issue(id, &request)
            .await
            .context("UserHandle::update_issue_status failed")?;
        Ok(())
    }

    /// Retrieve an issue by ID.
    pub async fn get_issue(&self, id: &IssueId) -> Result<IssueVersionRecord> {
        self.client
            .get_issue(id, false)
            .await
            .context("UserHandle::get_issue failed")
    }

    /// List all issues matching the default (empty) query.
    pub async fn list_issues(&self) -> Result<ListIssuesResponse> {
        self.client
            .list_issues(&SearchIssuesQuery::default())
            .await
            .context("UserHandle::list_issues failed")
    }

    // ── Patch operations ─────────────────────────────────────────────

    /// Create a new patch with the given title and description.
    /// Returns the new patch's ID.
    pub async fn create_patch(
        &self,
        title: &str,
        description: &str,
        repo: &RepoName,
    ) -> Result<PatchId> {
        let patch = Patch::new(
            title.to_string(),
            description.to_string(),
            String::new(), // empty diff
            PatchStatus::Open,
            false,
            None,
            Vec::new(),
            repo.clone(),
            None,
            false,
        );
        let request = UpsertPatchRequest::new(patch);
        let response = self
            .client
            .create_patch(&request)
            .await
            .context("UserHandle::create_patch failed")?;
        Ok(response.patch_id)
    }

    /// Retrieve a patch by ID.
    pub async fn get_patch(&self, id: &PatchId) -> Result<PatchVersionRecord> {
        self.client
            .get_patch(id)
            .await
            .context("UserHandle::get_patch failed")
    }

    /// Create a patch with GitHub PR metadata attached.
    ///
    /// Used for tests that exercise GitHub sync (review sync, merge flow).
    pub async fn create_patch_with_github(
        &self,
        title: &str,
        description: &str,
        repo: &RepoName,
        github_pr: GithubPr,
    ) -> Result<PatchId> {
        let patch = Patch::new(
            title.to_string(),
            description.to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            None,
            Vec::new(),
            repo.clone(),
            Some(github_pr),
            false,
        );
        let request = UpsertPatchRequest::new(patch);
        let response = self
            .client
            .create_patch(&request)
            .await
            .context("UserHandle::create_patch_with_github failed")?;
        Ok(response.patch_id)
    }

    /// List all patches matching the default (empty) query.
    pub async fn list_patches(&self) -> Result<ListPatchesResponse> {
        self.client
            .list_patches(&SearchPatchesQuery::default())
            .await
            .context("UserHandle::list_patches failed")
    }

    /// List jobs, optionally filtered by issue ID.
    pub async fn list_jobs_for_issue(
        &self,
        issue_id: &IssueId,
    ) -> Result<Vec<metis_common::jobs::JobVersionRecord>> {
        let query = SearchJobsQuery::new(None, Some(issue_id.clone()), None, None);
        let response = self
            .client
            .list_jobs(&query)
            .await
            .context("UserHandle::list_jobs_for_issue failed")?;
        Ok(response.jobs)
    }

    // ── Issue operations (extended) ──────────────────────────────────

    /// Create an issue with full control over type, status, assignee, and job settings.
    ///
    /// This is the lower-level variant of [`create_issue`](Self::create_issue)
    /// for tests that need to set specific job settings (e.g. repo_name, image,
    /// branch) or a specific assignee.
    pub async fn create_issue_with_settings(
        &self,
        description: &str,
        issue_type: IssueType,
        status: IssueStatus,
        assignee: Option<&str>,
        job_settings: Option<JobSettings>,
    ) -> Result<IssueId> {
        let issue = Issue::new(
            issue_type,
            description.to_string(),
            Username::from(self.name.as_str()),
            String::new(),
            status,
            assignee.map(|s| s.to_string()),
            job_settings,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            false,
        );
        let request = UpsertIssueRequest::new(issue, None);
        let response = self
            .client
            .create_issue(&request)
            .await
            .context("UserHandle::create_issue_with_settings failed")?;
        Ok(response.issue_id)
    }

    /// Create an issue with full control over all fields.
    ///
    /// This is the most flexible variant, exposing every field of the
    /// `Issue` type for tests that need complete control (e.g. setting
    /// dependencies, patches, or todo items at creation time).
    #[allow(clippy::too_many_arguments)]
    pub async fn create_issue_full(
        &self,
        issue_type: IssueType,
        description: &str,
        status: IssueStatus,
        assignee: Option<&str>,
        job_settings: Option<JobSettings>,
        dependencies: Vec<IssueDependency>,
        patches: Vec<PatchId>,
    ) -> Result<IssueId> {
        let issue = Issue::new(
            issue_type,
            description.to_string(),
            Username::from(self.name.as_str()),
            String::new(),
            status,
            assignee.map(|s| s.to_string()),
            job_settings,
            Vec::new(),
            dependencies,
            patches,
            false,
        );
        let request = UpsertIssueRequest::new(issue, None);
        let response = self
            .client
            .create_issue(&request)
            .await
            .context("UserHandle::create_issue_full failed")?;
        Ok(response.issue_id)
    }

    // ── Job operations ───────────────────────────────────────────────

    /// Create a job for the given repo with the given prompt.
    /// Returns the new job's task ID.
    pub async fn create_job(&self, repo: &RepoName, prompt: &str) -> Result<TaskId> {
        let request = CreateJobRequest::new(
            prompt.to_string(),
            None,
            BundleSpec::ServiceRepository {
                name: repo.clone(),
                rev: None,
            },
            HashMap::new(),
        );
        let response = self
            .client
            .create_job(&request)
            .await
            .context("UserHandle::create_job failed")?;
        Ok(response.job_id)
    }

    /// Create a job for the given repo, prompt, and issue.
    ///
    /// Like [`create_job`](Self::create_job) but links the job to the given
    /// issue, which sets the `spawned_from` field on the task. This ensures
    /// that `METIS_ISSUE_ID` is available for subprocess commands.
    pub async fn create_job_for_issue(
        &self,
        repo: &RepoName,
        prompt: &str,
        issue_id: &IssueId,
    ) -> Result<TaskId> {
        let mut request = CreateJobRequest::new(
            prompt.to_string(),
            None,
            BundleSpec::ServiceRepository {
                name: repo.clone(),
                rev: None,
            },
            HashMap::new(),
        );
        request.issue_id = Some(issue_id.clone());
        let response = self
            .client
            .create_job(&request)
            .await
            .context("UserHandle::create_job_for_issue failed")?;
        Ok(response.job_id)
    }

    // ── CLI operations ───────────────────────────────────────────────

    /// Execute the `metis` CLI binary as a subprocess with the given arguments.
    ///
    /// The subprocess is configured with the correct server URL and auth token
    /// via environment variables. Returns the captured output.
    ///
    /// Panics if the subprocess exits with a non-zero status.
    pub async fn cli(&self, args: &[&str]) -> Result<CliOutput> {
        let output = self.run_cli(args).await?;
        if !output.status.success() {
            anyhow::bail!(
                "metis CLI failed with status {}.\nstdout: {}\nstderr: {}",
                output.status,
                output.stdout,
                output.stderr,
            );
        }
        Ok(output)
    }

    /// Execute the `metis` CLI binary as a subprocess, expecting it to fail.
    ///
    /// Returns the captured output. Panics if the subprocess exits successfully.
    pub async fn cli_expect_failure(&self, args: &[&str]) -> Result<CliOutput> {
        let output = self.run_cli(args).await?;
        if output.status.success() {
            anyhow::bail!(
                "expected metis CLI to fail, but it succeeded.\nstdout: {}\nstderr: {}",
                output.stdout,
                output.stderr,
            );
        }
        Ok(output)
    }

    /// Internal helper that runs the CLI subprocess and captures output.
    async fn run_cli(&self, args: &[&str]) -> Result<CliOutput> {
        let metis_bin = env!("CARGO_BIN_EXE_metis");
        let output = Command::new(metis_bin)
            .args(args)
            .env("METIS_SERVER_URL", &self.server_url)
            .env("METIS_TOKEN", &self.token)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("failed to execute metis CLI subprocess")?;

        Ok(CliOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            status: output.status,
        })
    }
}
