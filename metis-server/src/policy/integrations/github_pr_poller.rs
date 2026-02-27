use crate::{
    AppState,
    background::scheduler::{ScheduledWorker, WorkerOutcome},
    domain::actors::ActorRef,
    domain::patches::{
        GithubCiFailure, GithubCiState, GithubCiStatus, GithubPr, Patch, PatchStatus, Review,
    },
};
use anyhow::Context;
use chrono::{DateTime, Duration, Utc};
use metis_common::api::v1 as api;
use metis_common::{PatchId, Versioned};
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
use std::{collections::HashSet, sync::Arc};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

const AUTHENTICATED_RATE_LIMIT_PER_HOUR: u64 = 5_000;
/// Conservative estimate: open patches use ~6 API calls (PR + CI status + CI checks +
/// reviews + review comments + issue comments), non-open patches use ~3 (PR + CI status
/// + CI checks). We use the higher value to stay within rate limits.
///
/// Non-open patches are bounded by a recency filter (see `NON_OPEN_PATCH_RECENCY`), so
/// the working set stays manageable.
const REQUESTS_PER_PATCH: u64 = 6;
/// Only sync CI status for non-open (Closed/Merged) patches that were updated within
/// this duration. This keeps the working set bounded as the total number of closed/merged
/// patches grows over time.
const NON_OPEN_PATCH_RECENCY: Duration = Duration::hours(1);
const WORKER_NAME: &str = "github_poller";

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
            match sync_patches(&self.state, self.max_patches_per_cycle, &mut start_from).await {
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

/// Returns true if a patch should be included in the sync cycle.
/// Open/ChangesRequested patches are always included; non-open patches are only
/// included if they were updated after `recency_cutoff`.
fn should_sync_patch(patch: &Versioned<Patch>, recency_cutoff: DateTime<Utc>) -> bool {
    let is_open = matches!(
        patch.item.status,
        PatchStatus::Open | PatchStatus::ChangesRequested
    );
    is_open || patch.timestamp >= recency_cutoff
}

async fn sync_patches(
    state: &AppState,
    max_per_cycle: usize,
    start_from: &mut usize,
) -> anyhow::Result<SyncStats> {
    let now = Utc::now();
    let recency_cutoff = now - NON_OPEN_PATCH_RECENCY;
    let github_patches: Vec<(PatchId, Versioned<Patch>)> = state
        .list_patches()
        .await?
        .into_iter()
        .filter(|(_, patch)| patch.item.github.is_some())
        .filter(|(_, patch)| should_sync_patch(patch, recency_cutoff))
        .collect();

    if github_patches.is_empty() {
        *start_from = 0;
        return Ok(SyncStats::default());
    }

    if *start_from >= github_patches.len() {
        *start_from = 0;
    }

    let mut ordered = Vec::with_capacity(github_patches.len());
    ordered.extend_from_slice(&github_patches[*start_from..]);
    if *start_from > 0 {
        ordered.extend_from_slice(&github_patches[..*start_from]);
    }

    let limit = max_per_cycle.max(1);
    let planned = github_patches.len().min(limit);
    info!(
        count = planned,
        total = github_patches.len(),
        "synchronizing GitHub patches"
    );

    let mut stats = SyncStats::default();
    let mut attempted = 0usize;
    for (patch_id, patch) in ordered.into_iter().take(limit) {
        match sync_patch_from_github(state, &patch_id, patch.item).await {
            Ok(()) => stats.processed += 1,
            Err(err) => {
                stats.failed += 1;
                warn!(patch_id = %patch_id, error = %err, "failed to sync patch from GitHub");
            }
        }

        attempted += 1;
    }

    *start_from = (*start_from + attempted) % github_patches.len();

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
    let Some(client) = select_github_installation_client(state, &github).await? else {
        warn!(
            patch_id = %patch_id,
            owner = %github.owner,
            repo = %github.repo,
            service_repo_name = %patch.service_repo_name,
            "skipping GitHub sync because no GitHub App installation token is available"
        );
        return Ok(());
    };

    // Note: `is_open` is derived from the snapshot passed into this function. The patch
    // status could change between now and the later re-fetch of `latest_patch`, but this
    // is harmless — worst case one cycle uses the wrong sync path (full vs CI-only).
    let is_open = matches!(
        patch.status,
        PatchStatus::Open | PatchStatus::ChangesRequested
    );

    let pr = client
        .pulls(&github.owner, &github.repo)
        .get(github.number)
        .await?;

    let ci_status = fetch_ci_status(&client, &github, &pr).await?;

    let latest_patch = state.get_patch(patch_id, false).await?;
    let latest_patch = latest_patch.item;
    if latest_patch.github.is_none() {
        debug!(patch_id = %patch_id, "skipping GitHub sync for patch without GitHub metadata");
        return Ok(());
    }

    let updated_patch = if is_open {
        // Full sync for open patches: fetch reviews and comments too.
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

        let github_reviews = build_review_entries(reviews, review_comments, issue_comments);
        let github_reviews = filter_reviews_by_creator(github_reviews, patch.creator.as_str());
        let review_updates = review_updates(&latest_patch.reviews, github_reviews);
        apply_github_sync(
            latest_patch.clone(),
            &github,
            &pr,
            review_updates,
            ci_status,
        )
    } else {
        // CI-only sync for non-open patches (Closed/Merged): skip review API calls.
        debug!(patch_id = %patch_id, status = ?patch.status, "CI-only sync for non-open patch");
        apply_ci_only_sync(latest_patch.clone(), &github, &pr, ci_status)
    };

    if updated_patch != latest_patch {
        state
            .upsert_patch(
                ActorRef::System {
                    worker_name: WORKER_NAME.into(),
                    on_behalf_of: None,
                },
                Some(patch_id.clone()),
                api::patches::UpsertPatchRequest::new(updated_patch.into()),
            )
            .await
            .with_context(|| format!("failed to persist GitHub sync for patch '{patch_id}'"))?;
        info!(patch_id = %patch_id, "updated patch from GitHub metadata");
    }

    Ok(())
}

async fn select_github_installation_client(
    state: &AppState,
    github: &GithubPr,
) -> anyhow::Result<Option<Octocrab>> {
    let Some(app_client) = state.github_app.as_ref() else {
        return Ok(None);
    };

    let installation = match app_client
        .apps()
        .get_repository_installation(&github.owner, &github.repo)
        .await
    {
        Ok(installation) => installation,
        Err(err) => {
            warn!(
                owner = %github.owner,
                repo = %github.repo,
                error = %err,
                "failed to lookup GitHub App installation"
            );
            return Ok(None);
        }
    };

    let (installation_client, _token) =
        match app_client.installation_and_token(installation.id).await {
            Ok(result) => result,
            Err(err) => {
                warn!(
                    owner = %github.owner,
                    repo = %github.repo,
                    installation_id = %installation.id,
                    error = %err,
                    "failed to fetch GitHub App installation token"
                );
                return Ok(None);
            }
        };

    Ok(Some(installation_client))
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

        entries.push(Review::new(
            body,
            review
                .state
                .as_ref()
                .map(|state| state == &octocrab::models::pulls::ReviewState::Approved)
                .unwrap_or(false),
            author,
            review.submitted_at,
        ));
    }

    for comment in review_comments {
        let body = comment.body.trim();
        if body.is_empty() {
            continue;
        }

        let Some(author) = comment.user.as_ref().map(|user| user.login.clone()) else {
            continue;
        };

        entries.push(Review::new(
            body.to_string(),
            false,
            author,
            Some(comment.created_at),
        ));
    }

    for comment in issue_comments {
        let Some(body) = comment.body.as_ref().map(|value| value.trim()) else {
            continue;
        };
        if body.is_empty() {
            continue;
        }

        entries.push(Review::new(
            body.to_string(),
            false,
            comment.user.login.clone(),
            Some(comment.created_at),
        ));
    }

    dedupe_reviews(entries)
}

fn filter_reviews_by_creator(reviews: Vec<Review>, creator: &str) -> Vec<Review> {
    let before_count = reviews.len();
    let filtered: Vec<Review> = reviews
        .into_iter()
        .filter(|review| review.author.eq_ignore_ascii_case(creator))
        .collect();
    let removed = before_count - filtered.len();
    if removed > 0 {
        debug!(
            creator = %creator,
            removed = removed,
            "filtered out reviews from non-creator authors"
        );
    }
    filtered
}

fn merge_reviews(existing: &[Review], github_reviews: Vec<Review>) -> Vec<Review> {
    let mut merged_reviews = Vec::new();
    let mut seen = HashSet::new();

    for review in github_reviews.into_iter().chain(existing.iter().cloned()) {
        let key = review_key(&review);
        if seen.insert(key) {
            merged_reviews.push(review);
        }
    }

    merged_reviews.sort_by_key(|review| {
        let timestamp = review
            .submitted_at
            .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap());
        (timestamp, review.author.clone())
    });

    merged_reviews
}

#[derive(Debug, Clone)]
struct ReviewUpdates {
    merged_reviews: Vec<Review>,
    has_new_changes_requested: bool,
}

fn review_updates(existing: &[Review], github_reviews: Vec<Review>) -> ReviewUpdates {
    let merged_reviews = merge_reviews(existing, github_reviews);
    let has_new_changes_requested = has_new_non_approved_reviews(existing, &merged_reviews);

    ReviewUpdates {
        merged_reviews,
        has_new_changes_requested,
    }
}

fn has_new_non_approved_reviews(existing: &[Review], github_reviews: &[Review]) -> bool {
    let existing_keys: HashSet<_> = existing.iter().map(review_key).collect();
    github_reviews
        .iter()
        .any(|review| !review.is_approved && !existing_keys.contains(&review_key(review)))
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

fn apply_github_sync(
    mut latest_patch: Patch,
    github: &GithubPr,
    pr: &PullRequest,
    review_updates: ReviewUpdates,
    ci_status: GithubCiStatus,
) -> Patch {
    let pr_status = patch_status_from_github(pr);
    let merged_reviews = review_updates.merged_reviews;
    let has_new_changes_requested = review_updates.has_new_changes_requested;
    let new_status = match pr_status {
        PatchStatus::Closed | PatchStatus::Merged => pr_status,
        PatchStatus::Open => {
            if has_new_changes_requested {
                PatchStatus::ChangesRequested
            } else {
                latest_patch.status
            }
        }
        PatchStatus::ChangesRequested => latest_patch.status,
    };

    latest_patch.reviews = merged_reviews;
    latest_patch.status = new_status;
    let mut updated_github = latest_patch
        .github
        .clone()
        .unwrap_or_else(|| github.clone());
    updated_github.head_ref = Some(pr.head.ref_field.clone());
    updated_github.base_ref = Some(pr.base.ref_field.clone());
    updated_github.url = pr.html_url.as_ref().map(ToString::to_string);
    updated_github.ci = Some(ci_status);
    latest_patch.github = Some(updated_github);

    latest_patch
}

fn apply_ci_only_sync(
    mut latest_patch: Patch,
    github: &GithubPr,
    pr: &PullRequest,
    ci_status: GithubCiStatus,
) -> Patch {
    let mut updated_github = latest_patch
        .github
        .clone()
        .unwrap_or_else(|| github.clone());
    updated_github.head_ref = Some(pr.head.ref_field.clone());
    updated_github.base_ref = Some(pr.base.ref_field.clone());
    updated_github.url = pr.html_url.as_ref().map(ToString::to_string);
    updated_github.ci = Some(ci_status);
    latest_patch.github = Some(updated_github);

    latest_patch
}

async fn fetch_ci_status(
    client: &Octocrab,
    github: &GithubPr,
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
        return GithubCiStatus::new(GithubCiState::Failed, Some(failure));
    }

    if check_runs.check_runs.iter().any(is_pending_check_run) {
        return GithubCiStatus::new(GithubCiState::Pending, None);
    }

    if let Some(failure) = first_failed_status(&combined_status.statuses) {
        return GithubCiStatus::new(GithubCiState::Failed, Some(failure));
    }

    if combined_status
        .statuses
        .iter()
        .any(|status| matches!(status.state, octocrab::models::StatusState::Pending))
    {
        return GithubCiStatus::new(GithubCiState::Pending, None);
    }

    GithubCiStatus::new(state_from_combined_status(&combined_status), None)
}

fn first_failed_check_run(check_runs: &[CheckRun]) -> Option<GithubCiFailure> {
    check_runs.iter().find_map(|run| {
        let conclusion = run.conclusion.as_deref()?;
        if is_failed_conclusion(conclusion) {
            return Some(GithubCiFailure::new(
                run.name.clone(),
                run.output.summary.clone(),
                run.details_url.clone().or_else(|| run.html_url.clone()),
            ));
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
            Some(GithubCiFailure::new(
                status
                    .context
                    .clone()
                    .unwrap_or_else(|| "status".to_string()),
                status.description.clone(),
                status.target_url.clone(),
            ))
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
    use metis_common::RepoName;
    use serde_json::json;
    use std::{str::FromStr, sync::Arc};

    use crate::domain::users::Username;
    use crate::test_utils::{FailingStore, test_state, test_state_handles, test_state_with_store};

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
        let handles = test_state_handles();
        handles
            .store
            .add_patch(
                Patch::new(
                    "test".to_string(),
                    "desc".to_string(),
                    sample_diff(),
                    PatchStatus::Open,
                    false,
                    None,
                    Username::from("test-creator"),
                    Vec::new(),
                    RepoName::from_str("dourolabs/api")?,
                    Some(GithubPr::new(
                        "octo".to_string(),
                        "repo".to_string(),
                        1,
                        None,
                        None,
                        None,
                        None,
                    )),
                    None,
                    None,
                    None,
                ),
                &ActorRef::test(),
            )
            .await?;
        let worker = GithubPollerWorker::new(handles.state, 60);

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
        let handles = test_state_with_store(Arc::new(FailingStore));
        let worker = GithubPollerWorker::new(handles.state, 60);

        let outcome = worker.run_iteration().await;

        assert!(matches!(outcome, WorkerOutcome::TransientError { .. }));
    }

    #[test]
    fn merge_reviews_preserves_existing() {
        let existing = vec![Review::new(
            "local".to_string(),
            false,
            "alice".to_string(),
            None,
        )];
        let github_reviews = vec![Review::new(
            "approved".to_string(),
            true,
            "bob".to_string(),
            Some(Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()),
        )];

        let merged_reviews = merge_reviews(&existing, github_reviews.clone());

        assert_eq!(merged_reviews.len(), 2);
        assert!(merged_reviews.contains(&github_reviews[0]));
        assert!(merged_reviews.contains(&existing[0]));
    }

    #[test]
    fn new_non_approved_reviews_trigger_changes_requested() {
        let existing = vec![Review::new(
            "looks fine".to_string(),
            true,
            "alice".to_string(),
            None,
        )];
        let github_reviews = vec![
            existing[0].clone(),
            Review::new("please update".to_string(), false, "bob".to_string(), None),
        ];

        assert!(has_new_non_approved_reviews(&existing, &github_reviews));

        let approvals_only = vec![Review::new(
            "lgtm".to_string(),
            true,
            "carol".to_string(),
            None,
        )];
        assert!(!has_new_non_approved_reviews(&existing, &approvals_only));
    }

    #[test]
    fn dedupe_reviews_removes_duplicates() {
        let review = Review::new(
            "same".to_string(),
            false,
            "alice".to_string(),
            Some(Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()),
        );
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
    fn apply_github_sync_marks_changes_requested_on_new_review() {
        let github = GithubPr::new(
            "octo".to_string(),
            "repo".to_string(),
            1,
            None,
            None,
            None,
            None,
        );
        let patch = Patch::new(
            "Patch".to_string(),
            "Patch description".to_string(),
            sample_diff(),
            PatchStatus::Open,
            false,
            None,
            Username::from("test-creator"),
            Vec::new(),
            RepoName::from_str("dourolabs/api").unwrap(),
            Some(github.clone()),
            None,
            None,
            None,
        );
        let pr: PullRequest = serde_json::from_value(json!({
            "url": "",
            "id": 1,
            "number": 1,
            "state": "open",
            "locked": false,
            "maintainer_can_modify": false,
            "html_url": "https://example.com/pr/1",
            "head": { "ref": "feature", "sha": "abc123", "user": null, "repo": null },
            "base": { "ref": "main", "sha": "def456", "user": null, "repo": null }
        }))
        .unwrap();
        let reviews = vec![Review::new(
            "please update".to_string(),
            false,
            "alice".to_string(),
            None,
        )];
        let ci_status = GithubCiStatus::new(GithubCiState::Success, None);

        let updated = apply_github_sync(
            patch,
            &github,
            &pr,
            review_updates(&[], reviews.clone()),
            ci_status.clone(),
        );

        assert_eq!(updated.status, PatchStatus::ChangesRequested);
        assert_eq!(updated.reviews, reviews);
        let updated_github = updated.github.expect("github metadata should be set");
        assert_eq!(updated_github.head_ref, Some("feature".to_string()));
        assert_eq!(updated_github.base_ref, Some("main".to_string()));
        assert_eq!(
            updated_github.url.as_deref(),
            Some("https://example.com/pr/1")
        );
        assert_eq!(updated_github.ci, Some(ci_status));
    }

    #[test]
    fn apply_github_sync_preserves_changes_requested_without_new_review() {
        let github = GithubPr::new(
            "octo".to_string(),
            "repo".to_string(),
            1,
            None,
            None,
            None,
            None,
        );
        let existing_reviews = vec![Review::new(
            "please update".to_string(),
            false,
            "alice".to_string(),
            None,
        )];
        let patch = Patch::new(
            "Patch".to_string(),
            "Patch description".to_string(),
            sample_diff(),
            PatchStatus::ChangesRequested,
            false,
            None,
            Username::from("test-creator"),
            existing_reviews.clone(),
            RepoName::from_str("dourolabs/api").unwrap(),
            Some(github.clone()),
            None,
            None,
            None,
        );
        let pr: PullRequest = serde_json::from_value(json!({
            "url": "",
            "id": 1,
            "number": 1,
            "state": "open",
            "locked": false,
            "maintainer_can_modify": false,
            "html_url": "https://example.com/pr/1",
            "head": { "ref": "feature", "sha": "abc123", "user": null, "repo": null },
            "base": { "ref": "main", "sha": "def456", "user": null, "repo": null }
        }))
        .unwrap();
        let ci_status = GithubCiStatus::new(GithubCiState::Success, None);

        let updated = apply_github_sync(
            patch,
            &github,
            &pr,
            review_updates(&existing_reviews, existing_reviews.clone()),
            ci_status.clone(),
        );

        assert_eq!(updated.status, PatchStatus::ChangesRequested);
        assert_eq!(updated.reviews, existing_reviews);
        let updated_github = updated.github.expect("github metadata should be set");
        assert_eq!(updated_github.head_ref, Some("feature".to_string()));
        assert_eq!(updated_github.base_ref, Some("main".to_string()));
        assert_eq!(
            updated_github.url.as_deref(),
            Some("https://example.com/pr/1")
        );
        assert_eq!(updated_github.ci, Some(ci_status));
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

    #[tokio::test]
    async fn select_github_installation_client_returns_none_without_app() {
        let state = test_state();
        let github = GithubPr::new(
            "octo".to_string(),
            "repo".to_string(),
            1,
            None,
            None,
            None,
            None,
        );

        let client = select_github_installation_client(&state, &github)
            .await
            .expect("select should not error without app");

        assert!(client.is_none());
    }

    fn make_github_user(login: &str) -> serde_json::Value {
        json!({
            "login": login,
            "id": 1,
            "node_id": "NODEID",
            "avatar_url": "https://example.com/avatar",
            "gravatar_id": "",
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

    fn make_pr_review(login: &str, body: &str, state: &str) -> PullRequestReview {
        serde_json::from_value(json!({
            "id": 101,
            "node_id": "NODEID",
            "html_url": "https://example.com/reviews/101",
            "user": make_github_user(login),
            "body": body,
            "state": state,
            "submitted_at": "2024-01-01T00:00:00Z",
            "pull_request_url": "https://example.com/pr/1"
        }))
        .unwrap()
    }

    fn make_pr_comment(login: &str, body: &str) -> PullRequestComment {
        serde_json::from_value(json!({
            "url": "https://api.example.com/repos/owner/repo/pulls/comments/1",
            "pull_request_review_id": null,
            "id": 1,
            "node_id": "NODEID",
            "diff_hunk": "@@ -1,3 +1,3 @@",
            "path": "README.md",
            "position": null,
            "original_position": null,
            "commit_id": "abc123",
            "original_commit_id": "abc123",
            "in_reply_to_id": null,
            "user": make_github_user(login),
            "body": body,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "html_url": "https://example.com/pr/comments/1",
            "author_association": null,
            "_links": {
                "self": { "href": "https://example.com" },
                "html": { "href": "https://example.com" },
                "pull_request": { "href": "https://example.com" }
            },
            "start_line": null,
            "original_start_line": null,
            "start_side": null,
            "line": null,
            "original_line": null,
            "side": null
        }))
        .unwrap()
    }

    fn make_issue_comment(login: &str, body: &str) -> IssueComment {
        serde_json::from_value(json!({
            "id": 1,
            "node_id": "NODEID",
            "url": "https://api.example.com/repos/owner/repo/issues/comments/1",
            "html_url": "https://example.com/issues/comments/1",
            "issue_url": null,
            "body": body,
            "body_text": null,
            "body_html": null,
            "author_association": null,
            "user": make_github_user(login),
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": null
        }))
        .unwrap()
    }

    #[test]
    fn build_review_entries_collects_all_review_types() {
        let reviews = vec![make_pr_review("alice", "looks good", "APPROVED")];
        let review_comments = vec![make_pr_comment("bob", "comment body")];
        let issue_comments = vec![make_issue_comment("charlie", "issue comment")];

        let result = build_review_entries(reviews, review_comments, issue_comments);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].author, "alice");
        assert!(result[0].is_approved);
        assert_eq!(result[1].author, "bob");
        assert_eq!(result[2].author, "charlie");
    }

    #[test]
    fn filter_reviews_by_creator_keeps_creator_reviews() {
        let reviews = vec![
            Review::new(
                "creator review".to_string(),
                false,
                "alice".to_string(),
                None,
            ),
            Review::new("third party".to_string(), false, "bob".to_string(), None),
        ];
        let result = filter_reviews_by_creator(reviews, "alice");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].author, "alice");
        assert_eq!(result[0].contents, "creator review");
    }

    #[test]
    fn filter_reviews_by_creator_case_insensitive() {
        let reviews = vec![
            Review::new("review 1".to_string(), false, "Alice".to_string(), None),
            Review::new("review 2".to_string(), false, "ALICE".to_string(), None),
            Review::new("review 3".to_string(), false, "aLiCe".to_string(), None),
        ];
        let result = filter_reviews_by_creator(reviews, "alice");
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn filter_reviews_by_creator_removes_all_third_party() {
        let reviews = vec![
            Review::new("review 1".to_string(), false, "bob".to_string(), None),
            Review::new("review 2".to_string(), false, "charlie".to_string(), None),
            Review::new("review 3".to_string(), false, "dave".to_string(), None),
        ];
        let result = filter_reviews_by_creator(reviews, "alice");
        assert!(result.is_empty());
    }

    #[test]
    fn build_and_filter_end_to_end() {
        let reviews = vec![
            make_pr_review("alice", "creator review", "COMMENTED"),
            make_pr_review("bob", "third party review", "CHANGES_REQUESTED"),
        ];
        let review_comments = vec![
            make_pr_comment("alice", "creator comment"),
            make_pr_comment("charlie", "third party comment"),
        ];
        let issue_comments = vec![
            make_issue_comment("alice", "creator issue comment"),
            make_issue_comment("dave", "third party issue comment"),
        ];

        let all_reviews = build_review_entries(reviews, review_comments, issue_comments);
        assert_eq!(all_reviews.len(), 6);

        let filtered = filter_reviews_by_creator(all_reviews, "alice");
        assert_eq!(filtered.len(), 3);
        assert!(
            filtered
                .iter()
                .all(|r| r.author.eq_ignore_ascii_case("alice"))
        );
    }

    #[tokio::test]
    async fn github_worker_includes_merged_patches_in_sync() -> anyhow::Result<()> {
        let handles = test_state_handles();
        handles
            .store
            .add_patch(
                Patch::new(
                    "test".to_string(),
                    "desc".to_string(),
                    sample_diff(),
                    PatchStatus::Merged,
                    false,
                    None,
                    Username::from("test-creator"),
                    Vec::new(),
                    RepoName::from_str("dourolabs/api")?,
                    Some(GithubPr::new(
                        "octo".to_string(),
                        "repo".to_string(),
                        1,
                        None,
                        None,
                        None,
                        None,
                    )),
                    None,
                    None,
                    None,
                ),
                &ActorRef::test(),
            )
            .await?;
        let worker = GithubPollerWorker::new(handles.state, 60);

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
    async fn github_worker_includes_closed_patches_in_sync() -> anyhow::Result<()> {
        let handles = test_state_handles();
        handles
            .store
            .add_patch(
                Patch::new(
                    "test".to_string(),
                    "desc".to_string(),
                    sample_diff(),
                    PatchStatus::Closed,
                    false,
                    None,
                    Username::from("test-creator"),
                    Vec::new(),
                    RepoName::from_str("dourolabs/api")?,
                    Some(GithubPr::new(
                        "octo".to_string(),
                        "repo".to_string(),
                        1,
                        None,
                        None,
                        None,
                        None,
                    )),
                    None,
                    None,
                    None,
                ),
                &ActorRef::test(),
            )
            .await?;
        let worker = GithubPollerWorker::new(handles.state, 60);

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

    #[test]
    fn should_sync_patch_includes_open_patches_regardless_of_age() {
        let now = Utc::now();
        let stale = now - Duration::hours(24);
        let cutoff = now - NON_OPEN_PATCH_RECENCY;

        for status in [PatchStatus::Open, PatchStatus::ChangesRequested] {
            let patch = Versioned::with_actor(
                Patch::new(
                    "test".to_string(),
                    "desc".to_string(),
                    sample_diff(),
                    status,
                    false,
                    None,
                    Username::from("test-creator"),
                    Vec::new(),
                    RepoName::from_str("dourolabs/api").unwrap(),
                    Some(GithubPr::new(
                        "octo".to_string(),
                        "repo".to_string(),
                        1,
                        None,
                        None,
                        None,
                        None,
                    )),
                    None,
                    None,
                    None,
                ),
                1,
                stale,
                ActorRef::test(),
                stale,
            );
            assert!(
                should_sync_patch(&patch, cutoff),
                "open/changes-requested patches should always be included"
            );
        }
    }

    #[test]
    fn should_sync_patch_includes_recent_non_open_patches() {
        let now = Utc::now();
        let recent = now - Duration::minutes(30);
        let cutoff = now - NON_OPEN_PATCH_RECENCY;

        for status in [PatchStatus::Merged, PatchStatus::Closed] {
            let patch = Versioned::with_actor(
                Patch::new(
                    "test".to_string(),
                    "desc".to_string(),
                    sample_diff(),
                    status,
                    false,
                    None,
                    Username::from("test-creator"),
                    Vec::new(),
                    RepoName::from_str("dourolabs/api").unwrap(),
                    Some(GithubPr::new(
                        "octo".to_string(),
                        "repo".to_string(),
                        1,
                        None,
                        None,
                        None,
                        None,
                    )),
                    None,
                    None,
                    None,
                ),
                1,
                recent,
                ActorRef::test(),
                recent,
            );
            assert!(
                should_sync_patch(&patch, cutoff),
                "recently-updated non-open patches should be included"
            );
        }
    }

    #[test]
    fn should_sync_patch_excludes_stale_non_open_patches() {
        let now = Utc::now();
        let stale = now - Duration::hours(2);
        let cutoff = now - NON_OPEN_PATCH_RECENCY;

        for status in [PatchStatus::Merged, PatchStatus::Closed] {
            let patch = Versioned::with_actor(
                Patch::new(
                    "test".to_string(),
                    "desc".to_string(),
                    sample_diff(),
                    status,
                    false,
                    None,
                    Username::from("test-creator"),
                    Vec::new(),
                    RepoName::from_str("dourolabs/api").unwrap(),
                    Some(GithubPr::new(
                        "octo".to_string(),
                        "repo".to_string(),
                        1,
                        None,
                        None,
                        None,
                        None,
                    )),
                    None,
                    None,
                    None,
                ),
                1,
                stale,
                ActorRef::test(),
                stale,
            );
            assert!(
                !should_sync_patch(&patch, cutoff),
                "stale non-open patches should be excluded"
            );
        }
    }

    #[test]
    fn apply_ci_only_sync_updates_ci_without_changing_status_or_reviews() {
        let existing_reviews = vec![Review::new(
            "please update".to_string(),
            false,
            "alice".to_string(),
            None,
        )];
        let github = GithubPr::new(
            "octo".to_string(),
            "repo".to_string(),
            1,
            None,
            None,
            None,
            None,
        );
        let patch = Patch::new(
            "Patch".to_string(),
            "Patch description".to_string(),
            sample_diff(),
            PatchStatus::Merged,
            false,
            None,
            Username::from("test-creator"),
            existing_reviews.clone(),
            RepoName::from_str("dourolabs/api").unwrap(),
            Some(github.clone()),
            None,
            None,
            None,
        );
        let pr: PullRequest = serde_json::from_value(json!({
            "url": "",
            "id": 1,
            "number": 1,
            "state": "closed",
            "locked": false,
            "maintainer_can_modify": false,
            "html_url": "https://example.com/pr/1",
            "head": { "ref": "feature", "sha": "abc123", "user": null, "repo": null },
            "base": { "ref": "main", "sha": "def456", "user": null, "repo": null }
        }))
        .unwrap();
        let ci_status = GithubCiStatus::new(
            GithubCiState::Failed,
            Some(GithubCiFailure::new(
                "build".to_string(),
                Some("compile error".to_string()),
                Some("https://ci.example.com/run/1".to_string()),
            )),
        );

        let updated = apply_ci_only_sync(patch.clone(), &github, &pr, ci_status.clone());

        // Status and reviews must remain unchanged.
        assert_eq!(updated.status, PatchStatus::Merged);
        assert_eq!(updated.reviews, existing_reviews);
        // CI status must be updated.
        let updated_github = updated.github.expect("github metadata should be set");
        assert_eq!(updated_github.ci, Some(ci_status));
        assert_eq!(updated_github.head_ref, Some("feature".to_string()));
        assert_eq!(updated_github.base_ref, Some("main".to_string()));
        assert_eq!(
            updated_github.url.as_deref(),
            Some("https://example.com/pr/1")
        );
    }
}
