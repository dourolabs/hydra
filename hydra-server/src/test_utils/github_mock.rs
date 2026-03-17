//! Builder for GitHub API mock servers used in integration tests.
//!
//! Reduces ~100 lines of GitHub API mock setup to ~5 lines per test by
//! providing a fluent builder API that configures all 8 GitHub endpoints
//! commonly needed: installation lookup, access token generation, PR details,
//! PR reviews, PR comments, issue comments, commit status, and commit check-runs.

use super::github_test_utils::github_user_response;
use anyhow::{Context, Result};
use httpmock::MockServer;
use httpmock::prelude::*;
use jsonwebtoken::EncodingKey;
use octocrab::Octocrab;
use octocrab::models::AppId;
use openssl::rsa::Rsa;
use serde_json::json;

/// A review configured on a mock PR.
pub struct MockReview {
    /// Review author login name.
    pub author: String,
    /// GitHub user ID for the review author.
    pub author_id: u64,
    /// Review state: `"APPROVED"`, `"CHANGES_REQUESTED"`, or `"COMMENTED"`.
    pub state: String,
    /// Review body text.
    pub body: String,
    /// Review ID used in the GitHub API response.
    pub id: u64,
    /// Timestamp for `submitted_at` in RFC 3339 format.
    pub submitted_at: String,
}

impl MockReview {
    /// Create a new review with the given author, state, and body.
    /// Uses sensible defaults for id, author_id, and submitted_at.
    pub fn new(
        author: impl Into<String>,
        state: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            author: author.into(),
            author_id: 1001,
            state: state.into(),
            body: body.into(),
            id: 101,
            submitted_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    /// Set the review ID.
    pub fn with_id(mut self, id: u64) -> Self {
        self.id = id;
        self
    }

    /// Set the author's GitHub user ID.
    pub fn with_author_id(mut self, author_id: u64) -> Self {
        self.author_id = author_id;
        self
    }

    /// Set the submitted_at timestamp.
    pub fn with_submitted_at(mut self, submitted_at: impl Into<String>) -> Self {
        self.submitted_at = submitted_at.into();
        self
    }
}

/// Configuration for a mock pull request.
pub struct MockPr {
    /// PR number.
    pub number: u64,
    /// PR state: `"open"` or `"closed"`.
    pub state: String,
    /// Whether the PR has been merged.
    pub merged: bool,
    /// The head branch ref name.
    pub head_ref: String,
    /// The head commit SHA.
    pub head_sha: String,
    /// Optional `merged_at` timestamp in RFC 3339 format.
    pub merged_at: Option<String>,
    /// Reviews attached to this PR (default: empty).
    pub reviews: Vec<MockReview>,
    /// PR review comments (default: empty JSON array).
    pub review_comments: serde_json::Value,
    /// Issue comments on the PR (default: empty JSON array).
    pub issue_comments: serde_json::Value,
    /// Commit status state (default: `"success"`).
    pub status_state: String,
    /// Check runs (default: empty).
    pub check_runs: serde_json::Value,
}

impl MockPr {
    /// Create a new open PR with the given number, head ref, and head SHA.
    pub fn new(number: u64, head_ref: impl Into<String>, head_sha: impl Into<String>) -> Self {
        Self {
            number,
            state: "open".to_string(),
            merged: false,
            head_ref: head_ref.into(),
            head_sha: head_sha.into(),
            merged_at: None,
            reviews: Vec::new(),
            review_comments: json!([]),
            issue_comments: json!([]),
            status_state: "success".to_string(),
            check_runs: json!({ "total_count": 0, "check_runs": [] }),
        }
    }

    /// Set the PR as merged.
    pub fn merged(mut self) -> Self {
        self.state = "closed".to_string();
        self.merged = true;
        self.merged_at = Some("2024-01-02T00:00:00Z".to_string());
        self
    }

    /// Set the PR as closed without merge.
    pub fn closed(mut self) -> Self {
        self.state = "closed".to_string();
        self.merged = false;
        self
    }

    /// Add a review to this PR.
    pub fn with_review(mut self, review: MockReview) -> Self {
        self.reviews.push(review);
        self
    }

    /// Set the reviews for this PR.
    pub fn with_reviews(mut self, reviews: Vec<MockReview>) -> Self {
        self.reviews = reviews;
        self
    }
}

struct MockInstallation {
    owner: String,
    repo: String,
    prs: Vec<MockPr>,
}

/// Builder for creating a mock GitHub API server and Octocrab client.
///
/// # Example
///
/// ```ignore
/// let (github_server, github_app) = GitHubMockBuilder::new()
///     .with_pr("octo", "repo", MockPr::new(99, "feature/review", &head_sha)
///         .with_review(MockReview::new("reviewer", "CHANGES_REQUESTED", "please update")))
///     .build()?;
/// ```
pub struct GitHubMockBuilder {
    installations: Vec<MockInstallation>,
}

impl GitHubMockBuilder {
    /// Create a new builder with no installations configured.
    pub fn new() -> Self {
        Self {
            installations: Vec::new(),
        }
    }

    /// Add a repository installation with default mocks (no PRs).
    pub fn with_installation(mut self, owner: impl Into<String>, repo: impl Into<String>) -> Self {
        let owner = owner.into();
        let repo = repo.into();
        // Only add if not already present
        if !self
            .installations
            .iter()
            .any(|i| i.owner == owner && i.repo == repo)
        {
            self.installations.push(MockInstallation {
                owner,
                repo,
                prs: Vec::new(),
            });
        }
        self
    }

    /// Configure a PR with specific state on a repository.
    /// Automatically adds the installation if not already present.
    pub fn with_pr(
        mut self,
        owner: impl Into<String>,
        repo: impl Into<String>,
        pr: MockPr,
    ) -> Self {
        let owner = owner.into();
        let repo = repo.into();
        if let Some(installation) = self
            .installations
            .iter_mut()
            .find(|i| i.owner == owner && i.repo == repo)
        {
            installation.prs.push(pr);
        } else {
            self.installations.push(MockInstallation {
                owner,
                repo,
                prs: vec![pr],
            });
        }
        self
    }

    /// Build the mock server and return `(MockServer, Octocrab)`.
    ///
    /// The `Octocrab` client is configured as a GitHub App pointing at the mock server.
    /// All configured installations and PRs are registered as mocks.
    pub fn build(self) -> Result<(MockServer, Octocrab)> {
        let server = MockServer::start();
        let base_url = server.base_url();
        let installation_id = 42u64;

        for installation in &self.installations {
            let owner = &installation.owner;
            let repo = &installation.repo;

            // Installation lookup
            server.mock(|when, then| {
                when.method(GET)
                    .path(format!("/repos/{owner}/{repo}/installation"));
                then.status(200).json_body(json!({
                    "id": installation_id,
                    "app_id": 1,
                    "account": github_user_response(owner, 1),
                    "repository_selection": "selected",
                    "access_tokens_url": format!(
                        "{}/app/installations/{}/access_tokens",
                        base_url, installation_id
                    ),
                    "repositories_url": format!("{}/installation/repositories", base_url),
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

            // Access token generation
            server.mock(|when, then| {
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

            // PR-specific mocks
            for pr in &installation.prs {
                let pr_number = pr.number;
                let head_sha = &pr.head_sha;

                // PR details
                server.mock(|when, then| {
                    when.method(GET)
                        .path(format!("/repos/{owner}/{repo}/pulls/{pr_number}"));
                    then.status(200).json_body(json!({
                        "url": "",
                        "id": pr_number,
                        "number": pr_number,
                        "state": pr.state,
                        "locked": false,
                        "maintainer_can_modify": false,
                        "html_url": format!("https://example.com/pr/{pr_number}"),
                        "merged": pr.merged,
                        "merged_at": pr.merged_at,
                        "head": { "ref": pr.head_ref, "sha": head_sha, "user": null, "repo": null },
                        "base": { "ref": "main", "sha": "def456", "user": null, "repo": null }
                    }));
                });

                // PR reviews
                let reviews_json: Vec<serde_json::Value> = pr
                    .reviews
                    .iter()
                    .map(|r| {
                        json!({
                            "id": r.id,
                            "node_id": "NODEID",
                            "html_url": format!("https://example.com/reviews/{}", r.id),
                            "body": r.body,
                            "state": r.state,
                            "user": github_user_response(&r.author, r.author_id),
                            "submitted_at": r.submitted_at,
                            "pull_request_url": format!("https://example.com/pr/{pr_number}")
                        })
                    })
                    .collect();
                server.mock(|when, then| {
                    when.method(GET)
                        .path(format!("/repos/{owner}/{repo}/pulls/{pr_number}/reviews"))
                        .query_param("per_page", "100");
                    then.status(200).json_body(json!(reviews_json));
                });

                // PR review comments
                let review_comments = pr.review_comments.clone();
                server.mock(|when, then| {
                    when.method(GET)
                        .path(format!("/repos/{owner}/{repo}/pulls/{pr_number}/comments"))
                        .query_param("per_page", "100");
                    then.status(200).json_body(review_comments);
                });

                // Issue comments
                let issue_comments = pr.issue_comments.clone();
                server.mock(|when, then| {
                    when.method(GET)
                        .path(format!("/repos/{owner}/{repo}/issues/{pr_number}/comments"))
                        .query_param("per_page", "100");
                    then.status(200).json_body(issue_comments);
                });

                // Commit status
                let status_state = pr.status_state.clone();
                let sha_for_status = head_sha.clone();
                server.mock(|when, then| {
                    when.method(GET).path(format!(
                        "/repos/{owner}/{repo}/commits/{sha_for_status}/status"
                    ));
                    then.status(200).json_body(json!({
                        "state": status_state,
                        "sha": sha_for_status,
                        "total_count": 0,
                        "statuses": []
                    }));
                });

                // Check runs
                let check_runs = pr.check_runs.clone();
                let sha_for_checks = head_sha.clone();
                server.mock(|when, then| {
                    when.method(GET)
                        .path(format!(
                            "/repos/{owner}/{repo}/commits/{sha_for_checks}/check-runs"
                        ))
                        .query_param("per_page", "100");
                    then.status(200).json_body(check_runs);
                });
            }
        }

        // Build Octocrab client pointing at the mock server
        let private_key =
            Rsa::generate(2048).context("failed to generate test RSA key for GitHubMockBuilder")?;
        let private_key_pem = private_key
            .private_key_to_pem()
            .context("failed to export test RSA key to PEM")?;
        let github_app = Octocrab::builder()
            .base_uri(&base_url)
            .context("failed to set mock GitHub base url")?
            .app(
                AppId::from(1),
                EncodingKey::from_rsa_pem(&private_key_pem)
                    .context("failed to parse test GitHub App key")?,
            )
            .build()
            .context("failed to build mock GitHub client")?;

        Ok((server, github_app))
    }
}

impl Default for GitHubMockBuilder {
    fn default() -> Self {
        Self::new()
    }
}
