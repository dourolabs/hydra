use crate::AppState;
use anyhow::Context;
use chrono::{DateTime, Utc};
use metis_common::{
    PatchId,
    github::{GithubClient, GithubConfig},
    patches::{
        GithubCiFailure, GithubCiState, GithubCiStatus, Patch, PatchStatus, Review,
        UpsertPatchRequest,
    },
};
use octocrab::{
    Octocrab,
    models::{
        CombinedStatus, Status,
        checks::{CheckRun, ListCheckRuns},
        issues::Comment as IssueComment,
        pulls::{Comment as PullRequestComment, PullRequest, Review as PullRequestReview},
    },
    params::repos::Commitish,
};
use std::collections::HashSet;
use tokio::time::{Duration, sleep};
use tracing::{debug, info, warn};

const AUTHENTICATED_RATE_LIMIT_PER_HOUR: u64 = 5_000;
const REQUESTS_PER_PATCH: u64 = 6;

/// Periodically polls GitHub for open patches linked to PRs and updates their status and reviews.
pub async fn poll_github_patches(state: AppState) {
    let scheduler = &state.config.background.scheduler.github_poller;
    let interval_secs = scheduler
        .interval_secs
        .max(state.config.background.github_poller.interval_secs);
    let sleep_duration = Duration::from_secs(interval_secs);
    let max_patches_per_cycle = max_patches_per_cycle(interval_secs);
    let mut start_from = 0usize;
    debug!(
        interval_secs,
        initial_backoff_secs = scheduler.initial_backoff_secs,
        max_backoff_secs = scheduler.max_backoff_secs,
        "github_poller scheduler configured"
    );

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
        if let Err(err) = sync_patch_from_github(state, &patch_id, patch).await {
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
    patch: Patch,
) -> anyhow::Result<()> {
    let Some(github) = patch.github.clone() else {
        return Ok(());
    };
    let Some(token) = select_github_token(state, patch.service_repo_name.as_deref()) else {
        warn!(
            patch_id = %patch_id,
            owner = %github.owner,
            repo = %github.repo,
            service_repo_name = ?patch.service_repo_name,
            "skipping GitHub sync because no GitHub token is configured"
        );
        return Ok(());
    };
    let github_client = github_client(&state.config.github, token)?;
    let per_page = github_client.per_page();
    let client = github_client.into_client();

    let pr = client
        .pulls(&github.owner, &github.repo)
        .get(github.number)
        .await?;
    let reviews = client
        .all_pages(
            client
                .pulls(&github.owner, &github.repo)
                .list_reviews(github.number)
                .per_page(per_page)
                .send()
                .await?,
        )
        .await?;
    let review_comments = client
        .all_pages(
            client
                .pulls(&github.owner, &github.repo)
                .list_comments(Some(github.number))
                .per_page(per_page)
                .send()
                .await?,
        )
        .await?;
    let issue_comments = client
        .all_pages(
            client
                .issues(&github.owner, &github.repo)
                .list_comments(github.number)
                .per_page(per_page)
                .send()
                .await?,
        )
        .await?;

    let github_reviews = build_review_entries(reviews, review_comments, issue_comments);
    let ci_status = fetch_ci_status(&client, &github, &pr, per_page).await?;

    let mut latest_patch = {
        let store = state.store.read().await;
        store.get_patch(patch_id).await?
    };
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
    updated_github.head_ref = Some(pr.head.ref_field.clone());
    updated_github.base_ref = Some(pr.base.ref_field.clone());
    updated_github.url = pr.html_url.as_ref().map(ToString::to_string);
    updated_github.ci = Some(ci_status);

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
        state
            .upsert_patch(
                Some(patch_id.clone()),
                UpsertPatchRequest {
                    patch: latest_patch,
                    job_id: None,
                },
            )
            .await
            .with_context(|| format!("failed to persist GitHub sync for patch '{patch_id}'"))?;
        info!(patch_id = %patch_id, "updated patch from GitHub metadata");
    }

    Ok(())
}

fn select_github_token(state: &AppState, service_repo_name: Option<&str>) -> Option<String> {
    if let Some(name) = service_repo_name {
        if let Some(repo) = state.service_state.repositories.get(name) {
            if let Some(token) = repo.github_token.as_deref().map(str::trim) {
                if !token.is_empty() {
                    return Some(token.to_string());
                }
            }
        }
    }

    state
        .config
        .github
        .token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
}

fn build_review_entries(
    reviews: Vec<PullRequestReview>,
    review_comments: Vec<PullRequestComment>,
    issue_comments: Vec<IssueComment>,
) -> Vec<Review> {
    let mut entries = Vec::new();

    for review in reviews {
        let Some(body) = review.body.as_ref().map(|value| value.trim().to_string()) else {
            continue;
        };
        if body.is_empty() {
            continue;
        }

        let Some(author) = review.user.as_ref().map(|user| user.login.clone()) else {
            continue;
        };

        entries.push(Review {
            contents: body,
            is_approved: review
                .state
                .as_ref()
                .map(|state| state == &octocrab::models::pulls::ReviewState::Approved)
                .unwrap_or(false),
            author,
            submitted_at: review.submitted_at,
        });
    }

    for comment in review_comments {
        let body = comment.body.trim();
        if body.is_empty() {
            continue;
        }

        let Some(author) = comment.user.as_ref().map(|user| user.login.clone()) else {
            continue;
        };

        entries.push(Review {
            contents: body.to_string(),
            is_approved: false,
            author,
            submitted_at: Some(comment.created_at),
        });
    }

    for comment in issue_comments {
        let Some(body) = comment.body.as_ref().map(|value| value.trim()) else {
            continue;
        };
        if body.is_empty() {
            continue;
        }

        entries.push(Review {
            contents: body.to_string(),
            is_approved: false,
            author: comment.user.login.clone(),
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
    if matches!(pr.state, Some(octocrab::models::IssueState::Open)) {
        PatchStatus::Open
    } else if pr.merged.unwrap_or(false) || pr.merged_at.is_some() {
        PatchStatus::Merged
    } else {
        PatchStatus::Closed
    }
}

fn github_client(config: &GithubConfig, token: String) -> anyhow::Result<GithubClient> {
    config
        .build_client_with_token(Some(token))
        .context("building GitHub client")
}

async fn fetch_ci_status(
    client: &Octocrab,
    github: &metis_common::patches::GithubPr,
    pr: &PullRequest,
    per_page: u8,
) -> anyhow::Result<GithubCiStatus> {
    let head_sha = pr.head.sha.clone();
    let combined_status: CombinedStatus = client
        .get(
            format!(
                "/repos/{owner}/{repo}/commits/{sha}/status",
                owner = github.owner,
                repo = github.repo,
                sha = head_sha
            ),
            None::<&()>,
        )
        .await
        .with_context(|| {
            format!(
                "fetching combined status for {}/{}@{}",
                github.owner, github.repo, head_sha
            )
        })?;

    let check_runs = client
        .checks(&github.owner, &github.repo)
        .list_check_runs_for_git_ref(Commitish(head_sha.clone()))
        .per_page(per_page)
        .send()
        .await
        .with_context(|| {
            format!(
                "fetching check runs for {}/{}@{}",
                github.owner, github.repo, head_sha
            )
        })?;

    Ok(ci_status_from_responses(check_runs, combined_status))
}

fn ci_status_from_responses(
    check_runs: ListCheckRuns,
    combined_status: CombinedStatus,
) -> GithubCiStatus {
    if let Some(failure) = first_failed_check_run(&check_runs.check_runs) {
        return GithubCiStatus {
            state: GithubCiState::Failed,
            failure: Some(failure),
        };
    }

    if check_runs.check_runs.iter().any(is_pending_check_run) {
        return GithubCiStatus {
            state: GithubCiState::Pending,
            failure: None,
        };
    }

    if let Some(failure) = first_failed_status(&combined_status.statuses) {
        return GithubCiStatus {
            state: GithubCiState::Failed,
            failure: Some(failure),
        };
    }

    if combined_status
        .statuses
        .iter()
        .any(|status| matches!(status.state, octocrab::models::StatusState::Pending))
    {
        return GithubCiStatus {
            state: GithubCiState::Pending,
            failure: None,
        };
    }

    GithubCiStatus {
        state: state_from_combined_status(&combined_status),
        failure: None,
    }
}

fn first_failed_check_run(check_runs: &[CheckRun]) -> Option<GithubCiFailure> {
    check_runs.iter().find_map(|run| {
        let conclusion = run.conclusion.as_deref()?;
        if is_failed_conclusion(conclusion) {
            return Some(GithubCiFailure {
                name: run.name.clone(),
                summary: run.output.summary.clone(),
                details_url: run.details_url.clone().or_else(|| run.html_url.clone()),
            });
        }

        None
    })
}

fn is_pending_check_run(run: &CheckRun) -> bool {
    run.conclusion.is_none()
}

fn is_failed_conclusion(conclusion: &str) -> bool {
    matches!(
        conclusion.to_ascii_lowercase().as_str(),
        "failure" | "cancelled" | "timed_out" | "action_required" | "startup_failure" | "stale"
    )
}

fn first_failed_status(statuses: &[Status]) -> Option<GithubCiFailure> {
    statuses.iter().find_map(|status| match status.state {
        octocrab::models::StatusState::Failure | octocrab::models::StatusState::Error => {
            Some(GithubCiFailure {
                name: status
                    .context
                    .clone()
                    .unwrap_or_else(|| "status".to_string()),
                summary: status.description.clone(),
                details_url: status.target_url.clone(),
            })
        }
        _ => None,
    })
}

fn state_from_combined_status(combined_status: &CombinedStatus) -> GithubCiState {
    match combined_status.state {
        octocrab::models::StatusState::Pending => GithubCiState::Pending,
        octocrab::models::StatusState::Failure | octocrab::models::StatusState::Error => {
            GithubCiState::Failed
        }
        octocrab::models::StatusState::Success => GithubCiState::Success,
        _ => GithubCiState::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;
    use std::{collections::HashMap, sync::Arc};

    use crate::{
        app::{GitRepository, ServiceState},
        test::test_state,
    };

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
        assert_eq!(max_patches_per_cycle(60), 13);
        assert_eq!(max_patches_per_cycle(120), 27);
    }

    #[test]
    fn patch_status_from_github_maps_states() {
        let mut base_pr: PullRequest = serde_json::from_value(json!({
            "url": "",
            "id": 1,
            "number": 1,
            "state": "open",
            "locked": false,
            "maintainer_can_modify": false,
            "head": { "ref": "feature", "sha": "", "user": null, "repo": null },
            "base": { "ref": "main", "sha": "", "user": null, "repo": null }
        }))
        .unwrap();

        assert!(matches!(
            patch_status_from_github(&base_pr),
            PatchStatus::Open
        ));

        base_pr.state = Some(octocrab::models::IssueState::Closed);
        base_pr.merged = Some(true);
        assert!(matches!(
            patch_status_from_github(&base_pr),
            PatchStatus::Merged
        ));

        base_pr.merged = Some(false);
        base_pr.merged_at = Some(Utc::now());
        assert!(matches!(
            patch_status_from_github(&base_pr),
            PatchStatus::Merged
        ));

        base_pr.merged = Some(false);
        base_pr.merged_at = None;
        assert!(matches!(
            patch_status_from_github(&base_pr),
            PatchStatus::Closed
        ));
    }

    #[test]
    fn ci_status_from_check_runs_marks_failure() {
        let check_runs = list_check_runs(vec![build_check_run(
            "build",
            Some("failure"),
            Some("compile error"),
            Some("https://ci.example.com/run/1"),
        )]);
        let combined = make_combined_status(
            "failure",
            vec![make_status(
                "failure",
                "build",
                Some("compile error"),
                Some("https://ci.example.com/run/1"),
            )],
        );

        let ci_status = ci_status_from_responses(check_runs, combined);

        assert!(matches!(ci_status.state, GithubCiState::Failed));
        let failure = ci_status.failure.expect("expected failure details");
        assert_eq!(failure.name, "build");
        assert_eq!(failure.summary.as_deref(), Some("compile error"));
        assert_eq!(
            failure.details_url.as_deref(),
            Some("https://ci.example.com/run/1")
        );
    }

    #[test]
    fn ci_status_from_check_runs_handles_pending() {
        let check_runs = list_check_runs(vec![build_check_run("tests", None, None, None)]);
        let combined =
            make_combined_status("pending", vec![make_status("pending", "tests", None, None)]);

        let ci_status = ci_status_from_responses(check_runs, combined);

        assert!(matches!(ci_status.state, GithubCiState::Pending));
        assert!(ci_status.failure.is_none());
    }

    #[test]
    fn ci_status_from_statuses_reports_success() {
        let check_runs = list_check_runs(vec![]);
        let combined = make_combined_status(
            "success",
            vec![make_status(
                "success",
                "lint",
                Some("ok"),
                Some("https://ci.example.com/lint"),
            )],
        );

        let ci_status = ci_status_from_responses(check_runs, combined);

        assert!(matches!(ci_status.state, GithubCiState::Success));
        assert!(ci_status.failure.is_none());
    }

    fn list_check_runs(runs: Vec<serde_json::Value>) -> ListCheckRuns {
        serde_json::from_value(json!({
            "total_count": runs.len(),
            "check_runs": runs,
        }))
        .unwrap()
    }

    fn build_check_run(
        name: &str,
        conclusion: Option<&str>,
        summary: Option<&str>,
        details_url: Option<&str>,
    ) -> serde_json::Value {
        json!({
            "id": 1,
            "node_id": format!("node-{name}"),
            "details_url": details_url,
            "head_sha": "abc123",
            "url": format!("https://api.example.com/checks/{name}"),
            "html_url": format!("https://ci.example.com/checks/{name}"),
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

    fn make_combined_status(state: &str, statuses: Vec<serde_json::Value>) -> CombinedStatus {
        serde_json::from_value(json!({
            "state": state,
            "sha": "abc123",
            "total_count": statuses.len(),
            "statuses": statuses,
            "repository": null,
            "commit_url": null,
            "url": null
        }))
        .unwrap()
    }

    fn make_status(
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

    #[test]
    fn select_github_token_requires_service_repo_name() {
        let state = test_state();

        assert!(select_github_token(&state, None).is_none());
    }

    #[test]
    fn select_github_token_uses_service_repo_name() {
        let mut state = test_state();
        state.service_state = Arc::new(ServiceState {
            repositories: HashMap::from([(
                "api".to_string(),
                GitRepository {
                    remote_url: "https://github.com/example/api.git".to_string(),
                    default_branch: None,
                    github_token: Some("svc-token".to_string()),
                    default_image: None,
                },
            )]),
        });

        let token = select_github_token(&state, Some("api"));

        assert_eq!(token.as_deref(), Some("svc-token"));
    }
}
