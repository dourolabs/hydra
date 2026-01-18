use crate::{
    AppState,
    background::scheduler::{ScheduledWorker, WorkerOutcome},
};
use anyhow::{Context, anyhow};
use chrono::{DateTime, Utc};
use metis_common::{
    PatchId, RepoName,
    issues::{Issue, IssueStatus, IssueType},
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
        pulls::{
            Comment as PullRequestComment, PullRequest, Review as PullRequestReview, ReviewAction,
        },
    },
    params::{pulls::State, repos::Commitish},
};
use serde_json::json;
use std::{collections::HashSet, sync::Arc};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

const AUTHENTICATED_RATE_LIMIT_PER_HOUR: u64 = 5_000;
const REQUESTS_PER_PATCH: u64 = 6;
const WORKER_NAME: &str = "github_poller";
const REVIEW_ASSIGNEE_PREFIX: &str = "review-assignee:";

#[derive(Clone)]
pub struct GithubPollerWorker {
    state: AppState,
    max_patches_per_cycle: usize,
    start_from: Arc<Mutex<usize>>,
}

impl GithubPollerWorker {
    pub fn new(state: AppState, interval_secs: u64) -> Self {
        let interval_secs = interval_secs.max(1);
        let max_patches_per_cycle = max_patches_per_cycle(interval_secs);

        Self {
            state,
            max_patches_per_cycle,
            start_from: Arc::new(Mutex::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl ScheduledWorker for GithubPollerWorker {
    async fn run_iteration(&self) -> WorkerOutcome {
        info!(worker = WORKER_NAME, "worker iteration started");
        let mut start_from = self.start_from.lock().await;

        let outcome =
            match sync_open_patches(&self.state, self.max_patches_per_cycle, &mut start_from).await
            {
                Ok(stats) if stats.processed == 0 && stats.failed == 0 => WorkerOutcome::Idle,
                Ok(stats) => WorkerOutcome::Progress {
                    processed: stats.processed,
                    failed: stats.failed,
                },
                Err(err) => WorkerOutcome::TransientError {
                    reason: err.to_string(),
                },
            };

        match &outcome {
            WorkerOutcome::Idle => info!(
                worker = WORKER_NAME,
                "no GitHub patches required syncing; worker idle"
            ),
            WorkerOutcome::Progress { processed, failed } => info!(
                worker = WORKER_NAME,
                processed, failed, "worker iteration completed successfully"
            ),
            WorkerOutcome::TransientError { reason } => info!(
                worker = WORKER_NAME,
                error = reason,
                "worker iteration completed with transient error"
            ),
        }

        outcome
    }
}

#[derive(Default)]
struct SyncStats {
    processed: usize,
    failed: usize,
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
) -> anyhow::Result<SyncStats> {
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
        return Ok(SyncStats::default());
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

    let mut stats = SyncStats::default();
    let mut attempted = 0usize;
    for (patch_id, patch) in ordered.into_iter().take(limit) {
        match sync_patch_from_github(state, &patch_id, patch).await {
            Ok(()) => stats.processed += 1,
            Err(err) => {
                stats.failed += 1;
                warn!(patch_id = %patch_id, error = %err, "failed to sync patch from GitHub");
            }
        }

        attempted += 1;
    }

    *start_from = (*start_from + attempted) % open_patches.len();

    Ok(stats)
}

async fn sync_patch_from_github(
    state: &AppState,
    patch_id: &PatchId,
    patch: Patch,
) -> anyhow::Result<()> {
    let Some(github) = patch.github.clone() else {
        return Ok(());
    };
    let Some(token) = select_github_token(state, &patch.service_repo_name).await else {
        warn!(
            patch_id = %patch_id,
            owner = %github.owner,
            repo = %github.repo,
            service_repo_name = %patch.service_repo_name,
            "skipping GitHub sync because no token is configured for the service repository"
        );
        return Ok(());
    };
    let client = github_client(token)?;

    let pr = client
        .pulls(&github.owner, &github.repo)
        .get(github.number)
        .await?;
    let reviews = client
        .all_pages(
            client
                .pulls(&github.owner, &github.repo)
                .list_reviews(github.number)
                .per_page(100)
                .send()
                .await?,
        )
        .await?;
    let review_comments = client
        .all_pages(
            client
                .pulls(&github.owner, &github.repo)
                .list_comments(Some(github.number))
                .per_page(100)
                .send()
                .await?,
        )
        .await?;
    let issue_comments = client
        .all_pages(
            client
                .issues(&github.owner, &github.repo)
                .list_comments(github.number)
                .per_page(100)
                .send()
                .await?,
        )
        .await?;

    let mut github_reviews = build_review_entries(reviews, review_comments, issue_comments);
    let ci_status = fetch_ci_status(&client, &github, &pr).await?;
    let mut new_status = patch_status_from_github(&pr);

    if let Err(err) = maybe_post_ci_failure_review_and_close(
        patch_id,
        &client,
        &github,
        &pr,
        &ci_status,
        &mut github_reviews,
        &mut new_status,
    )
    .await
    {
        warn!(
            patch_id = %patch_id,
            error = %err,
            "failed to post CI failure review or close PR"
        );
    }

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

    if let Err(err) =
        maybe_transition_ci_wait_issue(state, patch_id, &latest_patch, &ci_status).await
    {
        warn!(
            patch_id = %patch_id,
            error = %err,
            "failed to transition CI wait issue after CI success"
        );
    }

    let merged_reviews = merge_reviews(&latest_patch.reviews, github_reviews);
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

async fn maybe_post_ci_failure_review_and_close(
    patch_id: &PatchId,
    client: &Octocrab,
    github: &metis_common::patches::GithubPr,
    pr: &PullRequest,
    ci_status: &GithubCiStatus,
    reviews: &mut Vec<Review>,
    new_status: &mut PatchStatus,
) -> anyhow::Result<()> {
    let Some(failure) = failure_from_status(ci_status)? else {
        return Ok(());
    };

    if !matches!(patch_status_from_github(pr), PatchStatus::Open) {
        return Ok(());
    }
    if has_ci_failure_review(reviews, failure) {
        return Ok(());
    }

    let body = ci_failure_review_body(failure);
    let review: PullRequestReview = client
        .post(
            format!(
                "/repos/{owner}/{repo}/pulls/{number}/reviews",
                owner = github.owner,
                repo = github.repo,
                number = github.number
            ),
            Some(&json!({
                "body": body,
                "event": ReviewAction::Comment,
                "commit_id": pr.head.sha,
                "comments": []
            })),
        )
        .await
        .with_context(|| format!("posting CI failure review for patch '{patch_id}'"))?;
    reviews.push(Review {
        contents: body,
        is_approved: matches!(
            review.state,
            Some(octocrab::models::pulls::ReviewState::Approved)
        ),
        author: review
            .user
            .as_ref()
            .map(|user| user.login.clone())
            .unwrap_or_else(|| "metis".to_string()),
        submitted_at: review.submitted_at,
    });

    client
        .pulls(&github.owner, &github.repo)
        .update(github.number)
        .state(State::Closed)
        .send()
        .await
        .with_context(|| format!("closing PR for patch '{patch_id}' after CI failure"))?;
    *new_status = PatchStatus::Closed;

    Ok(())
}

fn failure_from_status(ci_status: &GithubCiStatus) -> anyhow::Result<Option<&GithubCiFailure>> {
    match ci_status.state {
        GithubCiState::Failed => ci_status
            .failure
            .as_ref()
            .map(Some)
            .ok_or_else(|| anyhow!("CI reported failed but no failure details were recorded")),
        _ => Ok(None),
    }
}

fn has_ci_failure_review(reviews: &[Review], failure: &GithubCiFailure) -> bool {
    let expected_body = ci_failure_review_body(failure);
    reviews
        .iter()
        .any(|review| review.contents == expected_body)
}

fn ci_failure_review_body(failure: &GithubCiFailure) -> String {
    let summary = failure
        .summary
        .as_deref()
        .unwrap_or("No summary was provided.");
    let logs = failure
        .details_url
        .as_deref()
        .unwrap_or("No logs URL was provided.");

    format!(
        "CI failed for this PR.\n\n\
         Failing check: {name}\n\
         Summary: {summary}\n\
         Logs: {logs}\n\n\
         Please re-merge `main`, fix CI locally, and push a new PR. Closing this PR to keep the queue clean.",
        name = failure.name,
    )
}

async fn maybe_transition_ci_wait_issue(
    state: &AppState,
    patch_id: &PatchId,
    patch: &Patch,
    ci_status: &GithubCiStatus,
) -> anyhow::Result<()> {
    if !matches!(ci_status.state, GithubCiState::Success) {
        return Ok(());
    }

    let previous_state = patch
        .github
        .as_ref()
        .and_then(|github| github.ci.as_ref())
        .map(|ci| ci.state);
    if matches!(previous_state, Some(GithubCiState::Success)) {
        return Ok(());
    }

    let mut store = state.store.write().await;
    let issue_ids = store
        .get_issues_for_patch(patch_id)
        .await
        .with_context(|| format!("fetching issues for patch '{patch_id}'"))?;

    if issue_ids.is_empty() {
        return Ok(());
    }

    let mut ci_wait_issues = Vec::new();
    let mut existing_review_issue = false;
    for issue_id in issue_ids {
        let issue = store
            .get_issue(&issue_id)
            .await
            .with_context(|| format!("fetching issue '{issue_id}'"))?;
        if let Some(review_assignee) = review_assignee_from_ci_wait_issue(&issue) {
            ci_wait_issues.push((issue_id, issue, review_assignee));
            continue;
        }
        if is_review_issue_for_patch(&issue, patch_id) {
            existing_review_issue = true;
        }
    }

    if ci_wait_issues.is_empty() {
        return Ok(());
    }

    let review_assignee = ci_wait_issues
        .iter()
        .find_map(|(_, _, assignee)| {
            let trimmed = assignee.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .ok_or_else(|| anyhow!("CI wait issue missing review assignee"))?;

    if !existing_review_issue {
        let description = review_issue_description(patch_id, patch);
        let (creator, dependencies) = ci_wait_issues
            .first()
            .map(|(_, issue, _)| (issue.creator.clone(), issue.dependencies.clone()))
            .unwrap_or_default();
        let review_issue = Issue {
            issue_type: IssueType::Task,
            description,
            creator,
            progress: String::new(),
            status: IssueStatus::Open,
            assignee: Some(review_assignee),
            dependencies,
            patches: vec![patch_id.clone()],
        };
        store
            .add_issue(review_issue)
            .await
            .with_context(|| format!("creating review issue for patch '{patch_id}'"))?;
    }

    for (issue_id, mut issue, _) in ci_wait_issues {
        if matches!(issue.status, IssueStatus::Closed | IssueStatus::Dropped) {
            continue;
        }
        issue.status = IssueStatus::Closed;
        store
            .update_issue(&issue_id, issue)
            .await
            .with_context(|| format!("closing CI wait issue '{issue_id}'"))?;
    }

    Ok(())
}

fn review_assignee_from_ci_wait_issue(issue: &Issue) -> Option<String> {
    if issue.assignee.as_deref() != Some("github") {
        return None;
    }
    parse_review_assignee(&issue.progress)
}

fn parse_review_assignee(progress: &str) -> Option<String> {
    progress
        .trim()
        .strip_prefix(REVIEW_ASSIGNEE_PREFIX)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn review_issue_description(patch_id: &PatchId, patch: &Patch) -> String {
    let summary = patch.title.trim();
    let title = if summary.is_empty() {
        patch
            .description
            .lines()
            .next()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .unwrap_or("Patch review")
            .to_string()
    } else {
        summary.to_string()
    };

    let mut description = format!("Review patch {patch_id}: {title}");
    if let Some(github) = patch.github.as_ref() {
        let pr_ref = github.url.clone().unwrap_or_else(|| {
            format!(
                "{owner}/{repo}#{number}",
                owner = github.owner,
                repo = github.repo,
                number = github.number
            )
        });
        description.push_str("\n\nPR: ");
        description.push_str(&pr_ref);
    }

    description
}

fn is_review_issue_for_patch(issue: &Issue, patch_id: &PatchId) -> bool {
    let expected_prefix = format!("Review patch {patch_id}");
    issue.description.trim_start().starts_with(&expected_prefix)
}

async fn select_github_token(state: &AppState, service_repo_name: &RepoName) -> Option<String> {
    state
        .service_state
        .repository(service_repo_name)
        .await
        .and_then(|repo| repo.github_token.clone())
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

fn github_client(token: String) -> anyhow::Result<Octocrab> {
    Octocrab::builder()
        .personal_token(token)
        .build()
        .context("building GitHub client")
}

async fn fetch_ci_status(
    client: &Octocrab,
    github: &metis_common::patches::GithubPr,
    pr: &PullRequest,
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
        .per_page(100)
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
    use metis_common::issues::{Issue, IssueStatus, IssueType};
    use metis_common::patches::GithubPr;
    use serde_json::json;
    use std::{collections::HashMap, str::FromStr, sync::Arc};
    use tokio::sync::RwLock;

    use crate::{
        app::{ServiceRepository, ServiceState},
        test_utils::{FailingStore, test_state},
    };

    fn sample_diff() -> String {
        "--- a/README.md\n+++ b/README.md\n@@\n-old\n+new\n".to_string()
    }

    #[tokio::test]
    async fn github_worker_returns_idle_without_open_patches() {
        let worker = GithubPollerWorker::new(test_state(), 60);

        assert_eq!(worker.run_iteration().await, WorkerOutcome::Idle);
    }

    #[tokio::test]
    async fn github_worker_reports_progress_for_github_patches_without_token() -> anyhow::Result<()>
    {
        let state = test_state();
        {
            let mut store = state.store.write().await;
            store
                .add_patch(Patch {
                    title: "test".to_string(),
                    description: "desc".to_string(),
                    diff: sample_diff(),
                    status: PatchStatus::Open,
                    is_automatic_backup: false,
                    reviews: Vec::new(),
                    service_repo_name: RepoName::from_str("dourolabs/api")?,
                    github: Some(GithubPr {
                        owner: "octo".to_string(),
                        repo: "repo".to_string(),
                        number: 1,
                        head_ref: None,
                        base_ref: None,
                        url: None,
                        ci: None,
                    }),
                })
                .await?;
        }
        let worker = GithubPollerWorker::new(state, 60);

        let outcome = worker.run_iteration().await;

        assert_eq!(
            outcome,
            WorkerOutcome::Progress {
                processed: 1,
                failed: 0
            }
        );

        Ok(())
    }

    #[tokio::test]
    async fn github_worker_surfaces_store_errors() {
        let mut state = test_state();
        state.store = Arc::new(RwLock::new(Box::new(FailingStore)));
        let worker = GithubPollerWorker::new(state, 60);

        let outcome = worker.run_iteration().await;

        assert!(matches!(outcome, WorkerOutcome::TransientError { .. }));
    }

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

    #[test]
    fn ci_failure_review_body_includes_details() {
        let failure = GithubCiFailure {
            name: "build".to_string(),
            summary: Some("compile error".to_string()),
            details_url: Some("https://ci.example.com/run/42".to_string()),
        };

        let body = ci_failure_review_body(&failure);

        assert!(body.contains("build"));
        assert!(body.contains("compile error"));
        assert!(body.contains("https://ci.example.com/run/42"));
        assert!(body.contains("re-merge `main`"));
        assert!(body.contains("push a new PR"));
    }

    #[test]
    fn has_ci_failure_review_detects_existing_body() {
        let failure = GithubCiFailure {
            name: "lint".to_string(),
            summary: Some("lint failed".to_string()),
            details_url: Some("https://ci.example.com/lint".to_string()),
        };
        let existing_body = ci_failure_review_body(&failure);
        let reviews = vec![Review {
            contents: existing_body.clone(),
            is_approved: false,
            author: "metis".to_string(),
            submitted_at: None,
        }];

        assert!(has_ci_failure_review(&reviews, &failure));

        let other_failure = GithubCiFailure {
            name: "tests".to_string(),
            summary: Some("tests failed".to_string()),
            details_url: Some("https://ci.example.com/tests".to_string()),
        };
        assert!(!has_ci_failure_review(&reviews, &other_failure));
    }

    #[test]
    fn failure_from_status_requires_failure_details() {
        let missing_details = GithubCiStatus {
            state: GithubCiState::Failed,
            failure: None,
        };
        assert!(failure_from_status(&missing_details).is_err());

        let success_status = GithubCiStatus {
            state: GithubCiState::Success,
            failure: None,
        };
        assert!(
            failure_from_status(&success_status)
                .expect("success state should be ok")
                .is_none()
        );
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

    #[tokio::test]
    async fn select_github_token_returns_none_for_unknown_repo() {
        let state = test_state();
        let repo_name = RepoName::from_str("dourolabs/api").unwrap();

        assert!(select_github_token(&state, &repo_name).await.is_none());
    }

    #[tokio::test]
    async fn select_github_token_uses_service_repo_name() {
        let mut state = test_state();
        let repo_name = RepoName::from_str("dourolabs/api").unwrap();
        state.service_state = Arc::new(ServiceState::with_repositories(HashMap::from([(
            repo_name.clone(),
            ServiceRepository {
                name: repo_name.clone(),
                remote_url: "https://github.com/dourolabs/api.git".to_string(),
                default_branch: None,
                github_token: Some("svc-token".to_string()),
                default_image: None,
            },
        )])));

        let token = select_github_token(&state, &repo_name).await;

        assert_eq!(token.as_deref(), Some("svc-token"));
    }

    #[tokio::test]
    async fn ci_pending_leaves_wait_issue_open_without_review_issue() -> anyhow::Result<()> {
        let state = test_state();
        let repo_name = RepoName::from_str("dourolabs/api")?;
        let patch = Patch {
            title: "Add feature".to_string(),
            description: "Adds feature".to_string(),
            diff: sample_diff(),
            status: PatchStatus::Open,
            is_automatic_backup: false,
            reviews: Vec::new(),
            service_repo_name: repo_name,
            github: Some(GithubPr {
                owner: "octo".to_string(),
                repo: "repo".to_string(),
                number: 41,
                head_ref: None,
                base_ref: None,
                url: Some("https://github.com/octo/repo/pull/41".to_string()),
                ci: Some(GithubCiStatus {
                    state: GithubCiState::Pending,
                    failure: None,
                }),
            }),
        };
        let patch_id = {
            let mut store = state.store.write().await;
            store.add_patch(patch.clone()).await?
        };
        {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: format!("Waiting on CI for patch {patch_id}: Add feature"),
                    creator: "metis".to_string(),
                    progress: "review-assignee: reviewer-a".to_string(),
                    status: IssueStatus::Open,
                    assignee: Some("github".to_string()),
                    dependencies: Vec::new(),
                    patches: vec![patch_id.clone()],
                })
                .await?;
        }

        let ci_status = GithubCiStatus {
            state: GithubCiState::Pending,
            failure: None,
        };
        maybe_transition_ci_wait_issue(&state, &patch_id, &patch, &ci_status).await?;

        let store = state.store.read().await;
        let issue_ids = store.get_issues_for_patch(&patch_id).await?;
        assert_eq!(issue_ids.len(), 1);
        let issue = store.get_issue(&issue_ids[0]).await?;
        assert_eq!(issue.assignee.as_deref(), Some("github"));
        assert_eq!(issue.status, IssueStatus::Open);

        Ok(())
    }

    #[tokio::test]
    async fn ci_success_closes_wait_issue_and_creates_review_issue() -> anyhow::Result<()> {
        let state = test_state();
        let repo_name = RepoName::from_str("dourolabs/api")?;
        let patch = Patch {
            title: "Add feature".to_string(),
            description: "Adds feature".to_string(),
            diff: sample_diff(),
            status: PatchStatus::Open,
            is_automatic_backup: false,
            reviews: Vec::new(),
            service_repo_name: repo_name,
            github: Some(GithubPr {
                owner: "octo".to_string(),
                repo: "repo".to_string(),
                number: 42,
                head_ref: None,
                base_ref: None,
                url: Some("https://github.com/octo/repo/pull/42".to_string()),
                ci: Some(GithubCiStatus {
                    state: GithubCiState::Pending,
                    failure: None,
                }),
            }),
        };
        let patch_id = {
            let mut store = state.store.write().await;
            store.add_patch(patch.clone()).await?
        };
        {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: format!("Waiting on CI for patch {patch_id}: Add feature"),
                    creator: "metis".to_string(),
                    progress: "review-assignee: reviewer-a".to_string(),
                    status: IssueStatus::Open,
                    assignee: Some("github".to_string()),
                    dependencies: Vec::new(),
                    patches: vec![patch_id.clone()],
                })
                .await?;
        }

        let ci_status = GithubCiStatus {
            state: GithubCiState::Success,
            failure: None,
        };
        maybe_transition_ci_wait_issue(&state, &patch_id, &patch, &ci_status).await?;

        let store = state.store.read().await;
        let issue_ids = store.get_issues_for_patch(&patch_id).await?;
        assert_eq!(issue_ids.len(), 2);

        let mut closed_wait_issue = false;
        let mut created_review_issue = false;
        for issue_id in issue_ids {
            let issue = store.get_issue(&issue_id).await?;
            if issue.assignee.as_deref() == Some("github") {
                assert_eq!(issue.status, IssueStatus::Closed);
                closed_wait_issue = true;
            }
            if issue.assignee.as_deref() == Some("reviewer-a") {
                assert_eq!(issue.status, IssueStatus::Open);
                assert!(issue.description.contains(patch_id.as_ref()));
                assert!(issue.description.contains("PR:"));
                created_review_issue = true;
            }
        }

        assert!(closed_wait_issue);
        assert!(created_review_issue);

        Ok(())
    }

    #[tokio::test]
    async fn ci_success_skips_duplicate_review_issue() -> anyhow::Result<()> {
        let state = test_state();
        let repo_name = RepoName::from_str("dourolabs/api")?;
        let patch = Patch {
            title: "Add feature".to_string(),
            description: "Adds feature".to_string(),
            diff: sample_diff(),
            status: PatchStatus::Open,
            is_automatic_backup: false,
            reviews: Vec::new(),
            service_repo_name: repo_name,
            github: Some(GithubPr {
                owner: "octo".to_string(),
                repo: "repo".to_string(),
                number: 43,
                head_ref: None,
                base_ref: None,
                url: Some("https://github.com/octo/repo/pull/43".to_string()),
                ci: Some(GithubCiStatus {
                    state: GithubCiState::Pending,
                    failure: None,
                }),
            }),
        };
        let patch_id = {
            let mut store = state.store.write().await;
            store.add_patch(patch.clone()).await?
        };
        {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: format!("Waiting on CI for patch {patch_id}: Add feature"),
                    creator: "metis".to_string(),
                    progress: "review-assignee: reviewer-a".to_string(),
                    status: IssueStatus::Open,
                    assignee: Some("github".to_string()),
                    dependencies: Vec::new(),
                    patches: vec![patch_id.clone()],
                })
                .await?;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: review_issue_description(&patch_id, &patch),
                    creator: "metis".to_string(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: Some("reviewer-a".to_string()),
                    dependencies: Vec::new(),
                    patches: vec![patch_id.clone()],
                })
                .await?;
        }

        let ci_status = GithubCiStatus {
            state: GithubCiState::Success,
            failure: None,
        };
        maybe_transition_ci_wait_issue(&state, &patch_id, &patch, &ci_status).await?;

        let store = state.store.read().await;
        let issue_ids = store.get_issues_for_patch(&patch_id).await?;
        assert_eq!(issue_ids.len(), 2);

        let mut closed_wait_issue = false;
        for issue_id in issue_ids {
            let issue = store.get_issue(&issue_id).await?;
            if issue.assignee.as_deref() == Some("github") {
                assert_eq!(issue.status, IssueStatus::Closed);
                closed_wait_issue = true;
            }
        }

        assert!(closed_wait_issue);

        Ok(())
    }
}
