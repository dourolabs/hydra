use crate::AppState;
use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use metis_common::{
    PatchId,
    constants::ENV_GH_TOKEN,
    patches::{GithubPr, Patch, PatchStatus, Review},
};
use reqwest::{
    Client, StatusCode,
    header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT},
};
use serde::Deserialize;
use std::{collections::HashSet, env};
use tokio::time::{Duration, sleep};
use tracing::{debug, info, warn};

const AUTHENTICATED_RATE_LIMIT_PER_HOUR: u64 = 5_000;
const REQUESTS_PER_PATCH: u64 = 4;

/// Periodically polls GitHub for open patches linked to PRs and updates their status and reviews.
pub async fn poll_github_patches(state: AppState) {
    let interval_secs = state.config.background.github_poller.interval_secs.max(60);
    let sleep_duration = Duration::from_secs(interval_secs);
    let max_patches_per_cycle = max_patches_per_cycle(interval_secs);
    let mut start_from = 0usize;

    loop {
        if let Err(err) = sync_open_patches(&state, max_patches_per_cycle, &mut start_from).await {
            warn!(error = %err, "failed to sync GitHub patches");
        }

        sleep(sleep_duration).await;
    }
}

fn max_patches_per_cycle(interval_secs: u64) -> usize {
    if interval_secs == 0 {
        return 1;
    }

    let cycles_per_hour = (3600f64 / interval_secs as f64).max(1.0);
    let patches =
        (AUTHENTICATED_RATE_LIMIT_PER_HOUR as f64 / REQUESTS_PER_PATCH as f64 / cycles_per_hour)
            .floor()
            .max(1.0);
    patches as usize
}

async fn sync_open_patches(
    state: &AppState,
    max_per_cycle: usize,
    start_from: &mut usize,
) -> anyhow::Result<()> {
    let open_patches: Vec<(PatchId, Patch)> = {
        let store = state.store.read().await;
        store
            .list_patches()
            .await?
            .into_iter()
            .filter(|(_, patch)| matches!(patch.status, PatchStatus::Open))
            .filter(|(_, patch)| patch.github.is_some())
            .collect()
    };

    if open_patches.is_empty() {
        *start_from = 0;
        return Ok(());
    }

    if *start_from >= open_patches.len() {
        *start_from = 0;
    }

    let mut ordered = Vec::with_capacity(open_patches.len());
    ordered.extend_from_slice(&open_patches[*start_from..]);
    if *start_from > 0 {
        ordered.extend_from_slice(&open_patches[..*start_from]);
    }

    let limit = max_per_cycle.max(1);
    let planned = open_patches.len().min(limit);
    info!(
        count = planned,
        total = open_patches.len(),
        "synchronizing GitHub patches"
    );
    let mut processed = 0usize;
    for (patch_id, patch) in ordered.into_iter().take(limit) {
        let Some(github) = patch.github.clone() else {
            processed += 1;
            continue;
        };

        if let Err(err) = sync_patch_from_github(state, &patch_id, github).await {
            warn!(patch_id = %patch_id, error = %err, "failed to sync patch from GitHub");
        }

        processed += 1;
    }

    *start_from = (*start_from + processed) % open_patches.len();

    Ok(())
}

async fn sync_patch_from_github(
    state: &AppState,
    patch_id: &PatchId,
    github: GithubPr,
) -> anyhow::Result<()> {
    let Some(token) = select_github_token(state, &github) else {
        warn!(
            patch_id = %patch_id,
            owner = %github.owner,
            repo = %github.repo,
            "skipping GitHub sync because no token is configured"
        );
        return Ok(());
    };
    let client = GithubClient::new(Some(token))?;

    let pr = client
        .pull_request(&github.owner, &github.repo, github.number)
        .await?;
    let reviews = client
        .pull_request_reviews(&github.owner, &github.repo, github.number)
        .await?;
    let review_comments = client
        .pull_request_review_comments(&github.owner, &github.repo, github.number)
        .await?;
    let issue_comments = client
        .issue_comments(&github.owner, &github.repo, github.number)
        .await?;

    let github_reviews = build_review_entries(reviews, review_comments, issue_comments);

    let mut store = state.store.write().await;
    let mut latest_patch = store.get_patch(patch_id).await?;
    if !matches!(latest_patch.status, PatchStatus::Open) {
        debug!(patch_id = %patch_id, "skipping GitHub sync for non-open patch");
        return Ok(());
    }
    if latest_patch.github.is_none() {
        debug!(patch_id = %patch_id, "skipping GitHub sync for patch without GitHub metadata");
        return Ok(());
    }

    let merged_reviews = merge_reviews(&latest_patch.reviews, github_reviews);
    let new_status = patch_status_from_github(&pr);
    let mut updated_github = latest_patch
        .github
        .clone()
        .unwrap_or_else(|| github.clone());
    updated_github.head_ref = Some(pr.head.ref_name);
    updated_github.base_ref = Some(pr.base.ref_name);
    updated_github.url = pr.html_url.or(updated_github.url);

    let mut changed = false;
    if merged_reviews != latest_patch.reviews {
        latest_patch.reviews = merged_reviews;
        changed = true;
    }
    if new_status != latest_patch.status {
        latest_patch.status = new_status;
        changed = true;
    }
    if latest_patch.github.as_ref() != Some(&updated_github) {
        latest_patch.github = Some(updated_github);
        changed = true;
    }

    if changed {
        store
            .update_patch(patch_id, latest_patch)
            .await
            .with_context(|| format!("failed to persist GitHub sync for patch '{patch_id}'"))?;
        info!(patch_id = %patch_id, "updated patch from GitHub metadata");
    }

    Ok(())
}

fn select_github_token(state: &AppState, github: &GithubPr) -> Option<String> {
    let repo_identifier = format!(
        "{}/{}",
        github.owner.to_lowercase(),
        github.repo.to_lowercase()
    );
    if let Some((_, repo)) = state.service_state.repositories.iter().find(|(_, repo)| {
        let remote = repo.remote_url.to_lowercase();
        remote.contains("github.com") && remote.contains(&repo_identifier)
    }) {
        if let Some(token) = &repo.github_token {
            return Some(token.clone());
        }
    }

    env::var(ENV_GH_TOKEN)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn build_review_entries(
    reviews: Vec<PullRequestReview>,
    review_comments: Vec<PullRequestComment>,
    issue_comments: Vec<PullRequestComment>,
) -> Vec<Review> {
    let mut entries = Vec::new();

    for review in reviews {
        let Some(body) = review.body.map(|value| value.trim().to_string()) else {
            continue;
        };
        if body.is_empty() {
            continue;
        }

        entries.push(Review {
            contents: body,
            is_approved: review.state.eq_ignore_ascii_case("APPROVED"),
            author: review.user.login,
            submitted_at: review.submitted_at,
        });
    }

    for comment in review_comments
        .into_iter()
        .chain(issue_comments.into_iter())
    {
        let Some(body) = comment.body.map(|value| value.trim().to_string()) else {
            continue;
        };
        if body.is_empty() {
            continue;
        }

        entries.push(Review {
            contents: body,
            is_approved: false,
            author: comment.user.login,
            submitted_at: Some(comment.created_at),
        });
    }

    dedupe_reviews(entries)
}

fn merge_reviews(existing: &[Review], github_reviews: Vec<Review>) -> Vec<Review> {
    let mut merged = Vec::new();
    let mut seen = HashSet::new();

    for review in github_reviews.into_iter().chain(existing.iter().cloned()) {
        let key = review_key(&review);
        if seen.insert(key) {
            merged.push(review);
        }
    }

    merged.sort_by_key(|review| {
        let timestamp = review
            .submitted_at
            .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap());
        (timestamp, review.author.clone())
    });

    merged
}

fn dedupe_reviews(reviews: Vec<Review>) -> Vec<Review> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();

    for review in reviews {
        let key = review_key(&review);
        if seen.insert(key) {
            unique.push(review);
        }
    }

    unique
}

fn review_key(review: &Review) -> (String, bool, String, Option<DateTime<Utc>>) {
    (
        review.author.clone(),
        review.is_approved,
        review.contents.clone(),
        review.submitted_at,
    )
}

fn patch_status_from_github(pr: &PullRequest) -> PatchStatus {
    if pr.state.eq_ignore_ascii_case("open") {
        PatchStatus::Open
    } else if pr.merged_at.is_some() {
        PatchStatus::Merged
    } else {
        PatchStatus::Closed
    }
}

#[derive(Debug, Deserialize, Clone)]
struct PullRequest {
    state: String,
    merged_at: Option<DateTime<Utc>>,
    head: PullRequestBranch,
    base: PullRequestBranch,
    html_url: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct PullRequestBranch {
    #[serde(rename = "ref")]
    ref_name: String,
}

#[derive(Debug, Deserialize, Clone)]
struct PullRequestReview {
    user: GithubUser,
    body: Option<String>,
    state: String,
    submitted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, Clone)]
struct PullRequestComment {
    user: GithubUser,
    body: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Clone)]
struct GithubUser {
    login: String,
}

struct GithubClient {
    client: Client,
}

impl GithubClient {
    fn new(token: Option<String>) -> anyhow::Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static("metis-server"));
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            "X-GitHub-Api-Version",
            HeaderValue::from_static("2022-11-28"),
        );
        if let Some(token) = token {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {token}"))?,
            );
        }

        let client = Client::builder().default_headers(headers).build()?;
        Ok(Self { client })
    }

    async fn pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> anyhow::Result<PullRequest> {
        self.get(&format!(
            "https://api.github.com/repos/{owner}/{repo}/pulls/{number}"
        ))
        .await
    }

    async fn pull_request_reviews(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> anyhow::Result<Vec<PullRequestReview>> {
        self.get_paginated(&format!(
            "https://api.github.com/repos/{owner}/{repo}/pulls/{number}/reviews"
        ))
        .await
    }

    async fn pull_request_review_comments(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> anyhow::Result<Vec<PullRequestComment>> {
        self.get_paginated(&format!(
            "https://api.github.com/repos/{owner}/{repo}/pulls/{number}/comments"
        ))
        .await
    }

    async fn issue_comments(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> anyhow::Result<Vec<PullRequestComment>> {
        self.get_paginated(&format!(
            "https://api.github.com/repos/{owner}/{repo}/issues/{number}/comments"
        ))
        .await
    }

    async fn get_paginated<T: for<'de> Deserialize<'de>>(
        &self,
        base_url: &str,
    ) -> anyhow::Result<Vec<T>> {
        let mut results = Vec::new();
        let mut page = 1;

        loop {
            let mut page_results: Vec<T> = self
                .get(&format!("{base_url}?per_page=100&page={page}"))
                .await?;
            let batch_len = page_results.len();
            results.append(&mut page_results);

            if batch_len < 100 {
                break;
            }

            page += 1;
        }

        Ok(results)
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, url: &str) -> anyhow::Result<T> {
        let response = self.client.get(url).send().await?;

        if response.status() == StatusCode::NOT_MODIFIED {
            bail!("received 304 Not Modified for {url}");
        }

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("GitHub request failed with {status}: {body}");
        }

        if let Some(remaining) = response
            .headers()
            .get("X-RateLimit-Remaining")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<i64>().ok())
        {
            debug!(url = %url, remaining, "GitHub API remaining rate limit");
        }

        Ok(response.json().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn merge_reviews_preserves_existing() {
        let existing = vec![Review {
            contents: "local".to_string(),
            is_approved: false,
            author: "alice".to_string(),
            submitted_at: None,
        }];
        let github_reviews = vec![Review {
            contents: "approved".to_string(),
            is_approved: true,
            author: "bob".to_string(),
            submitted_at: Some(Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()),
        }];

        let merged = merge_reviews(&existing, github_reviews.clone());

        assert_eq!(merged.len(), 2);
        assert!(merged.contains(&github_reviews[0]));
        assert!(merged.contains(&existing[0]));
    }

    #[test]
    fn dedupe_reviews_removes_duplicates() {
        let review = Review {
            contents: "same".to_string(),
            is_approved: false,
            author: "alice".to_string(),
            submitted_at: Some(Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()),
        };
        let result = dedupe_reviews(vec![review.clone(), review.clone()]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], review);
    }

    #[test]
    fn max_patches_per_cycle_respects_rate_limit() {
        assert_eq!(max_patches_per_cycle(60), 20);
        assert_eq!(max_patches_per_cycle(120), 41);
    }

    #[test]
    fn patch_status_from_github_maps_states() {
        let base_pr = PullRequest {
            state: "open".to_string(),
            merged_at: None,
            head: PullRequestBranch {
                ref_name: "feature".to_string(),
            },
            base: PullRequestBranch {
                ref_name: "main".to_string(),
            },
            html_url: None,
        };
        assert!(matches!(
            patch_status_from_github(&base_pr),
            PatchStatus::Open
        ));

        let merged_pr = PullRequest {
            state: "closed".to_string(),
            merged_at: Some(Utc::now()),
            ..base_pr.clone()
        };
        assert!(matches!(
            patch_status_from_github(&merged_pr),
            PatchStatus::Merged
        ));

        let closed_pr = PullRequest {
            state: "closed".to_string(),
            merged_at: None,
            ..base_pr
        };
        assert!(matches!(
            patch_status_from_github(&closed_pr),
            PatchStatus::Closed
        ));
    }
}
