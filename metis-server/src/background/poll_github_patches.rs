use crate::{
    AppState,
    background::scheduler::{ScheduledWorker, WorkerOutcome},
    domain::issues::{
        Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType, UpsertIssueRequest,
    },
    domain::patches::{
        GithubCiFailure, GithubCiState, GithubCiStatus, GithubPr, Patch, PatchStatus, Review,
        UpsertPatchRequest,
    },
    domain::users::Username,
};
use anyhow::Context;
use chrono::{DateTime, Utc};
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
const REQUESTS_PER_PATCH: u64 = 6;
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
    let open_patches: Vec<(PatchId, Versioned<Patch>)> = state
        .list_patches()
        .await?
        .into_iter()
        .filter(|(_, patch)| {
            matches!(
                patch.item.status,
                PatchStatus::Open | PatchStatus::ChangesRequested
            )
        })
        .filter(|(_, patch)| patch.item.github.is_some())
        .collect();

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
        match sync_patch_from_github(state, &patch_id, patch.item).await {
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

    let github_reviews = build_review_entries(reviews, review_comments, issue_comments);

    let latest_patch = state.get_patch(patch_id).await?;
    let latest_patch = latest_patch.item;
    if !matches!(
        latest_patch.status,
        PatchStatus::Open | PatchStatus::ChangesRequested
    ) {
        debug!(patch_id = %patch_id, "skipping GitHub sync for non-open patch");
        return Ok(());
    }
    if latest_patch.github.is_none() {
        debug!(patch_id = %patch_id, "skipping GitHub sync for patch without GitHub metadata");
        return Ok(());
    }

    let ci_status = fetch_ci_status(&client, &github, &pr).await?;
    let review_updates = review_updates(&latest_patch.reviews, github_reviews);
    let has_new_changes_requested = review_updates.has_new_changes_requested;
    let updated_patch = apply_github_sync(
        latest_patch.clone(),
        &github,
        &pr,
        review_updates,
        ci_status,
    );

    if updated_patch != latest_patch {
        state
            .upsert_patch(
                None,
                Some(patch_id.clone()),
                UpsertPatchRequest::new(updated_patch),
            )
            .await
            .with_context(|| format!("failed to persist GitHub sync for patch '{patch_id}'"))?;
        info!(patch_id = %patch_id, "updated patch from GitHub metadata");
    }

    create_followup_issues_on_new_review(state, patch_id, has_new_changes_requested).await?;

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
        PatchStatus::Open | PatchStatus::ChangesRequested => {
            if has_new_changes_requested {
                PatchStatus::ChangesRequested
            } else {
                PatchStatus::Open
            }
        }
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

async fn create_followup_issues_on_new_review(
    state: &AppState,
    patch_id: &PatchId,
    has_new_changes_requested: bool,
) -> anyhow::Result<()> {
    if !has_new_changes_requested {
        return Ok(());
    }

    let followup_agent = state.config.background.merge_request_followup_agent.trim();
    if followup_agent.is_empty() {
        warn!(patch_id = %patch_id, "merge_request_followup_agent not configured; skipping followup issue creation");
        return Ok(());
    }

    let issues = state.list_issues().await?;
    let mut created_issue_ids = Vec::new();

    for (issue_id, issue) in issues {
        let issue = issue.item;
        if issue.issue_type != IssueType::MergeRequest {
            continue;
        }
        if !matches!(issue.status, IssueStatus::Open | IssueStatus::InProgress) {
            continue;
        }
        if !issue.patches.contains(patch_id) {
            continue;
        }

        let followup_issue = Issue::new(
            IssueType::MergeRequest,
            format!("Follow-up for review on patch {patch_id}"),
            Username::from(""),
            String::new(),
            IssueStatus::Open,
            Some(followup_agent.to_string()),
            Some(issue.job_settings.clone()),
            Vec::new(),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                issue_id.clone(),
            )],
            issue.patches.clone(),
        );

        let followup_id = state
            .upsert_issue(None, UpsertIssueRequest::new(followup_issue, None))
            .await?;
        created_issue_ids.push((issue_id, followup_id));
    }

    if !created_issue_ids.is_empty() {
        let issues = created_issue_ids
            .iter()
            .map(|(parent_id, child_id)| format!("{parent_id}->{child_id}"))
            .collect::<Vec<_>>()
            .join(", ");
        info!(patch_id = %patch_id, issues = %issues, "created followup issues for new review");
    }

    Ok(())
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
    use crate::domain::issues::{Issue, IssueStatus, IssueType, JobSettings};
    use crate::domain::users::Username;
    use chrono::TimeZone;
    use metis_common::RepoName;
    use serde_json::json;
    use std::{str::FromStr, sync::Arc};

    use crate::test_utils::{FailingStore, test_state, test_state_handles, test_state_with_store};

    fn sample_diff() -> String {
        "--- a/README.md\n+++ b/README.md\n@@\n-old\n+new\n".to_string()
    }

    fn job_settings(repo_name: &RepoName) -> JobSettings {
        JobSettings {
            repo_name: Some(repo_name.clone()),
            ..JobSettings::default()
        }
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
            .add_patch(Patch::new(
                "test".to_string(),
                "desc".to_string(),
                sample_diff(),
                PatchStatus::Open,
                false,
                None,
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
            ))
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
            Vec::new(),
            RepoName::from_str("dourolabs/api").unwrap(),
            Some(github.clone()),
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
    fn apply_github_sync_resets_changes_requested_without_new_review() {
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
            existing_reviews.clone(),
            RepoName::from_str("dourolabs/api").unwrap(),
            Some(github.clone()),
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

        assert_eq!(updated.status, PatchStatus::Open);
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

    #[tokio::test]
    async fn followup_issue_created_for_open_merge_request_on_new_review() -> anyhow::Result<()> {
        let handles = test_state_handles();
        let repo_name = RepoName::from_str("dourolabs/api")?;
        let patch_id = handles
            .store
            .add_patch(Patch::new(
                "Patch".to_string(),
                "Patch description".to_string(),
                sample_diff(),
                PatchStatus::Open,
                false,
                None,
                Vec::new(),
                repo_name.clone(),
                None,
            ))
            .await?;

        let parent_issue_id = handles
            .store
            .add_issue(Issue::new(
                IssueType::MergeRequest,
                "Review patch".to_string(),
                Username::from("creator"),
                String::new(),
                IssueStatus::Open,
                Some("pm".to_string()),
                Some(job_settings(&repo_name)),
                Vec::new(),
                Vec::new(),
                vec![patch_id.clone()],
            ))
            .await?;

        create_followup_issues_on_new_review(&handles.state, &patch_id, true).await?;

        let children = handles.store.get_issue_children(&parent_issue_id).await?;
        assert_eq!(children.len(), 1);
        let child = handles.store.get_issue(&children[0]).await?.item;
        assert_eq!(child.issue_type, IssueType::MergeRequest);
        assert_eq!(child.status, IssueStatus::Open);
        assert_eq!(child.assignee.as_deref(), Some("swe"));
        assert_eq!(child.patches, vec![patch_id.clone()]);
        assert_eq!(child.job_settings.repo_name.as_ref(), Some(&repo_name));

        Ok(())
    }

    #[tokio::test]
    async fn followup_issue_skipped_without_new_review() -> anyhow::Result<()> {
        let handles = test_state_handles();
        let repo_name = RepoName::from_str("dourolabs/api")?;
        let patch_id = handles
            .store
            .add_patch(Patch::new(
                "Patch".to_string(),
                "Patch description".to_string(),
                sample_diff(),
                PatchStatus::Open,
                false,
                None,
                Vec::new(),
                repo_name.clone(),
                None,
            ))
            .await?;

        let parent_issue_id = handles
            .store
            .add_issue(Issue::new(
                IssueType::MergeRequest,
                "Review patch".to_string(),
                Username::from("creator"),
                String::new(),
                IssueStatus::Open,
                Some("pm".to_string()),
                Some(job_settings(&repo_name)),
                Vec::new(),
                Vec::new(),
                vec![patch_id.clone()],
            ))
            .await?;

        create_followup_issues_on_new_review(&handles.state, &patch_id, false).await?;

        let children = handles.store.get_issue_children(&parent_issue_id).await?;
        assert!(children.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn followup_issue_skipped_for_closed_merge_request() -> anyhow::Result<()> {
        let handles = test_state_handles();
        let repo_name = RepoName::from_str("dourolabs/api")?;
        let patch_id = handles
            .store
            .add_patch(Patch::new(
                "Patch".to_string(),
                "Patch description".to_string(),
                sample_diff(),
                PatchStatus::Open,
                false,
                None,
                Vec::new(),
                repo_name.clone(),
                None,
            ))
            .await?;

        let parent_issue_id = handles
            .store
            .add_issue(Issue::new(
                IssueType::MergeRequest,
                "Review patch".to_string(),
                Username::from("creator"),
                String::new(),
                IssueStatus::Closed,
                Some("pm".to_string()),
                Some(job_settings(&repo_name)),
                Vec::new(),
                Vec::new(),
                vec![patch_id.clone()],
            ))
            .await?;

        create_followup_issues_on_new_review(&handles.state, &patch_id, true).await?;

        let children = handles.store.get_issue_children(&parent_issue_id).await?;
        assert!(children.is_empty());

        Ok(())
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
}
