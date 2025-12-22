use std::{collections::HashMap, time::Duration};

use anyhow::{anyhow, Context, Result};
use octocrab::{models::pulls::PullRequest, models::IssueState, Octocrab};
use tokio::time::sleep;

use super::AsyncOp;

const DEFAULT_POLL_INTERVAL_SECS: u64 = 5;
const MAX_POLL_ATTEMPTS: u32 = 120;

pub(super) fn create_pull_request(
    owner: String,
    repo: String,
    title: String,
    head: String,
    base: String,
    body: Option<String>,
    continuation: rhai::FnPtr,
) -> (AsyncOp, rhai::FnPtr) {
    (
        AsyncOp::GithubCreatePullRequest {
            owner,
            repo,
            title,
            head,
            base,
            body,
        },
        continuation,
    )
}

pub(super) fn wait_for_pull_request(
    owner: String,
    repo: String,
    number: i64,
    continuation: rhai::FnPtr,
) -> (AsyncOp, rhai::FnPtr) {
    (
        AsyncOp::GithubWaitForPullRequest {
            owner,
            repo,
            number,
            poll_interval_secs: DEFAULT_POLL_INTERVAL_SECS,
        },
        continuation,
    )
}

fn github_token(env: &HashMap<String, String>) -> Result<String> {
    env.get("GH_TOKEN")
        .cloned()
        .ok_or_else(|| anyhow!("GH_TOKEN must be set in env"))
}

fn github_client(env: &HashMap<String, String>) -> Result<Octocrab> {
    let token = github_token(env)?;
    Octocrab::builder()
        .personal_token(token)
        .build()
        .context("failed to build GitHub client")
}

pub(super) async fn evaluate_create_pull_request(
    owner: &str,
    repo: &str,
    title: &str,
    head: &str,
    base: &str,
    body: &Option<String>,
    env: &HashMap<String, String>,
) -> Result<String> {
    let client = github_client(env)?;
    let pulls = client.pulls(owner, repo);
    let mut builder = pulls.create(title, head, base);
    if let Some(body) = body {
        builder = builder.body(body);
    }

    let pr = builder
        .send()
        .await
        .context("failed to create pull request via GitHub API")?;

    pull_request_status("created", &pr)
}

pub(super) async fn evaluate_wait_for_pull_request(
    owner: &str,
    repo: &str,
    number: i64,
    poll_interval_secs: u64,
    env: &HashMap<String, String>,
) -> Result<String> {
    let pr_number =
        u64::try_from(number).map_err(|_| anyhow!("pull request number must be non-negative"))?;
    let client = github_client(env)?;
    let pulls = client.pulls(owner, repo);
    let mut attempts: u32 = 0;

    loop {
        let pr = pulls
            .get(pr_number)
            .await
            .with_context(|| format!("failed to fetch pull request #{pr_number}"))?;

        if is_merged(&pr) {
            return pull_request_status("merged", &pr);
        }

        if matches!(pr.state, Some(IssueState::Closed)) {
            return pull_request_status("closed", &pr);
        }

        attempts += 1;
        if attempts >= MAX_POLL_ATTEMPTS {
            return Err(anyhow!(
                "timed out waiting for pull request #{pr_number} to merge or close"
            ));
        }

        sleep(Duration::from_secs(poll_interval_secs)).await;
    }
}

fn is_merged(pr: &PullRequest) -> bool {
    pr.merged == Some(true) || pr.merged_at.is_some()
}

fn pull_request_status(status: &str, pr: &PullRequest) -> Result<String> {
    let state = pr
        .state
        .as_ref()
        .map(issue_state_as_str)
        .unwrap_or("unknown");
    let summary = serde_json::json!({
        "number": pr.number,
        "status": status,
        "state": state,
        "url": pr.html_url.as_ref().map(|url| url.to_string()),
        "title": pr.title,
    });

    serde_json::to_string(&summary).context("failed to serialize pull request status")
}

fn issue_state_as_str(state: &IssueState) -> &'static str {
    match state {
        IssueState::Open => "open",
        IssueState::Closed => "closed",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_token_returns_error_for_create() {
        let env = HashMap::new();
        let result =
            evaluate_create_pull_request("owner", "repo", "title", "head", "base", &None, &env)
                .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_token_returns_error_for_wait() {
        let env = HashMap::new();
        let result =
            evaluate_wait_for_pull_request("owner", "repo", 1, DEFAULT_POLL_INTERVAL_SECS, &env)
                .await;

        assert!(result.is_err());
    }
}
