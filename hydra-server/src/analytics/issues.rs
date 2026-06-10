use super::buckets::{bin_index_for, bucket_starts, empty_duration_histogram, percentile, step};
use crate::app::projects::{ResolveStatusError, project_cached};
use crate::domain::issues::Issue;
use crate::store::{ReadOnlyStore, StoreError};
use chrono::{DateTime, Utc};
use hydra_common::api::v1::analytics::{
    BucketGranularity, IssueOverTimeBucket, IssuesCycleTimeResponse, IssuesOverTimeResponse,
    IssuesPerStatusDistributionResponse, IssuesThroughputQuery,
    IssuesTimeInStatusBreakdownResponse, PerStatusDistribution, TimeInStatusSegment,
};
use hydra_common::api::v1::issues::SearchIssuesQuery;
use hydra_common::api::v1::projects::{Project, StatusDefinition, StatusKey};
use hydra_common::principal::Principal;
use hydra_common::{IssueId, ProjectId, Versioned};
use std::collections::HashMap;
use std::str::FromStr;

/// One issue and its full ascending-version-order history. Aggregation
/// inputs are intentionally simple structs so unit tests can pass
/// hand-rolled fixtures without touching a Store.
#[derive(Debug, Clone)]
pub struct IssueHistory {
    pub issue_id: IssueId,
    pub versions: Vec<Versioned<Issue>>,
}

impl IssueHistory {
    pub fn new(issue_id: IssueId, versions: Vec<Versioned<Issue>>) -> Self {
        Self { issue_id, versions }
    }

    /// Creation timestamp (first version's `timestamp`).
    fn created_at(&self) -> DateTime<Utc> {
        self.versions
            .first()
            .expect("issue history must contain at least one version")
            .timestamp
    }

    fn project_id(&self) -> &ProjectId {
        &self
            .versions
            .first()
            .expect("issue history must contain at least one version")
            .item
            .project_id
    }

    /// First version whose status maps to a terminal definition (per
    /// [`Project`]). Returns `(status_key, timestamp)`. `None` if the
    /// issue never reached a terminal status, or if its status key is
    /// not declared in the project (treated as still-blocking).
    fn first_terminal<'a>(
        &self,
        project: &'a Project,
    ) -> Option<(&'a StatusDefinition, DateTime<Utc>)> {
        for version in &self.versions {
            let Some(def) = project.find_status(&version.item.status) else {
                continue;
            };
            if def.unblocks_parents {
                return Some((def, version.timestamp));
            }
        }
        None
    }
}

/// Apply the in-process filters that don't map cleanly to
/// [`SearchIssuesQuery`]: `repo_name` (lives in `session_settings`),
/// `assignee` (compared as the canonical Principal path string), and
/// the issue-type filter (`issue_types` plural include-set with
/// fallback to the singular `issue_type`). The store-side filters
/// (`creator`, `project_id`) are pushed down through
/// [`SearchIssuesQuery`] in [`fetch_issue_histories`]; this is the
/// leftover sieve.
fn issue_passes_inprocess_filters(issue: &Issue, query: &IssuesThroughputQuery) -> bool {
    if let Some(expected_repo) = query.repo_name.as_deref() {
        let repo_matches = issue
            .session_settings
            .repo_name
            .as_ref()
            .map(|r| r.to_string() == expected_repo)
            .unwrap_or(false);
        if !repo_matches {
            return false;
        }
    }
    if !query.issue_types.is_empty() {
        let issue_domain_type = &issue.issue_type;
        let matches = query.issue_types.iter().any(|wire_type| {
            // Unknown is a forward-compat sentinel with no domain
            // mapping; skip it so it never matches a real issue.
            if matches!(wire_type, hydra_common::api::v1::issues::IssueType::Unknown) {
                return false;
            }
            let dt: crate::domain::issues::IssueType = (*wire_type).into();
            &dt == issue_domain_type
        });
        if !matches {
            return false;
        }
    } else if let Some(expected_type) = query.issue_type {
        let domain_type: crate::domain::issues::IssueType = expected_type.into();
        if issue.issue_type != domain_type {
            return false;
        }
    }
    if let Some(expected_assignee) = query.assignee.as_deref() {
        let matches = match Principal::from_str(expected_assignee) {
            Ok(parsed) => issue
                .assignee
                .as_ref()
                .map(|a| hydra_common::principal::principal_eq(a, &parsed))
                .unwrap_or(false),
            Err(_) => false,
        };
        if !matches {
            return false;
        }
    }
    true
}

/// Fetch the issues matching the throughput-query filters and their
/// full version histories. Deleted issues are excluded by
/// `list_issues`'s default (we don't pass `include_deleted`).
/// `creator` and `project_id` are pushed into [`SearchIssuesQuery`];
/// `repo_name`, `issue_type`, and `assignee` are applied in process
/// because they don't map onto the store-side filter set today.
pub async fn fetch_issue_histories(
    store: &dyn ReadOnlyStore,
    query: &IssuesThroughputQuery,
) -> Result<Vec<IssueHistory>, StoreError> {
    let mut search = SearchIssuesQuery::default();
    search.project_id = query.project_id.clone();
    search.creator = query.creator.clone();

    let issues = store.list_issues(&search).await?;
    let mut histories = Vec::with_capacity(issues.len());
    for (issue_id, latest) in issues {
        if !issue_passes_inprocess_filters(&latest.item, query) {
            continue;
        }
        let versions = store.get_issue_versions(&issue_id).await?;
        if versions.is_empty() {
            continue;
        }
        histories.push(IssueHistory::new(issue_id, versions));
    }
    Ok(histories)
}

/// Resolve the `Project`s referenced by the supplied issue histories,
/// returning a `(ProjectId, Project)` cache. Failed lookups are skipped
/// — the calling aggregator treats issues with no resolvable project as
/// if their status weren't declared (no terminal flip, no time-in-status
/// contribution). This matches the conservative posture in the
/// `is_terminal` helper of `policy/restrictions/issue_lifecycle.rs`.
pub async fn resolve_projects_for_histories(
    store: &dyn ReadOnlyStore,
    histories: &[IssueHistory],
) -> Result<HashMap<ProjectId, Project>, StoreError> {
    let mut cache: HashMap<ProjectId, Project> = HashMap::new();
    for history in histories {
        let pid = history.project_id();
        if cache.contains_key(pid) {
            continue;
        }
        match project_cached(&mut cache, store, pid).await {
            Ok(_) => {}
            Err(ResolveStatusError::ProjectNotFound(_)) => {}
            Err(ResolveStatusError::Store(err)) => return Err(err),
            Err(other) => {
                tracing::warn!(error = %other, project_id = %pid, "analytics: skipping unresolvable project");
            }
        }
    }
    Ok(cache)
}

/// Returns `true` iff `history` should be included in the cohort under
/// the supplied `status_keys` include-filter. Empty `status_keys`
/// imposes no filter (all issues pass). Otherwise the issue must have
/// reached a terminal status (per the project's definitions) whose key
/// is in the set.
fn issue_passes_status_filter(
    history: &IssueHistory,
    project: &Project,
    status_keys: &[StatusKey],
) -> bool {
    if status_keys.is_empty() {
        return true;
    }
    history
        .first_terminal(project)
        .map(|(def, _)| status_keys.contains(&def.key))
        .unwrap_or(false)
}

/// Compute `issues/cycle_time`: histogram of `created_at → terminal_at`
/// for issues that reached a terminal status (per their *own* project's
/// status definitions) within `[from, to)`.
///
/// `status_keys`: when populated, only issues whose terminal status key
/// is in the set count toward the cohort (include-form filter).
pub fn compute_issues_cycle_time(
    histories: &[IssueHistory],
    projects: &HashMap<ProjectId, Project>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    status_keys: &[StatusKey],
) -> IssuesCycleTimeResponse {
    let mut deltas: Vec<u64> = Vec::new();
    let mut histogram = empty_duration_histogram();
    for history in histories {
        let Some(project) = projects.get(history.project_id()) else {
            continue;
        };
        let Some((_, terminal_at)) = history.first_terminal(project) else {
            continue;
        };
        if terminal_at < from || terminal_at >= to {
            continue;
        }
        if !issue_passes_status_filter(history, project, status_keys) {
            continue;
        }
        let created = history.created_at();
        let delta = (terminal_at - created).num_seconds().max(0) as u64;
        deltas.push(delta);
        let idx = bin_index_for(delta);
        histogram[idx].count += 1;
    }
    deltas.sort_unstable();
    let count = deltas.len() as u64;
    let median = percentile(&deltas, 0.5);
    let p95 = percentile(&deltas, 0.95);
    IssuesCycleTimeResponse::new(median, p95, count, histogram)
}

/// Compute `issues/over_time`: per-bucket counts of issues created and
/// of issues that reached terminal status (each per the issue's own
/// project's status definitions).
///
/// `status_keys`: when populated, gates only the `reached_terminal`
/// increment on the issue's terminal-status key being in the set.
/// `created` counts remain unfiltered — an issue's `created_at`
/// predates any terminal flip, so filtering creation on a future
/// status would surprise callers.
pub fn compute_issues_over_time(
    histories: &[IssueHistory],
    projects: &HashMap<ProjectId, Project>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    bucket: BucketGranularity,
    status_keys: &[StatusKey],
) -> IssuesOverTimeResponse {
    let starts = bucket_starts(from, to, bucket);
    if starts.is_empty() {
        return IssuesOverTimeResponse::new(Vec::new());
    }
    let step = step(bucket);

    let mut buckets: Vec<IssueOverTimeBucket> = starts
        .iter()
        .map(|s| IssueOverTimeBucket::new(*s, 0, 0))
        .collect();

    let first_start = starts[0];
    let bucket_len = buckets.len();
    let bucket_for = |t: DateTime<Utc>| -> Option<usize> {
        if t < from || t >= to {
            return None;
        }
        let delta = t - first_start;
        let idx = (delta.num_seconds() / step.num_seconds()) as usize;
        if idx >= bucket_len { None } else { Some(idx) }
    };

    for history in histories {
        let created = history.created_at();
        if let Some(idx) = bucket_for(created) {
            buckets[idx].created += 1;
        }
        let Some(project) = projects.get(history.project_id()) else {
            continue;
        };
        if let Some((_, terminal_at)) = history.first_terminal(project) {
            if !issue_passes_status_filter(history, project, status_keys) {
                continue;
            }
            if let Some(idx) = bucket_for(terminal_at) {
                buckets[idx].reached_terminal += 1;
            }
        }
    }

    IssuesOverTimeResponse::new(buckets)
}

/// Compute `issues/time_in_status_breakdown` for a single project's
/// status set: per-status mean time issues in the terminal-window cohort
/// spent in that status. The final terminal version contributes 0
/// (no time-in-status since it's the end-state) — matching the spec.
///
/// Cohort = issues whose terminal-status flip falls inside `[from, to)`.
/// `histories` is assumed to already be scoped to `project_id` — the
/// route handler enforces `project_id` is required for this endpoint.
///
/// `status_keys`: when populated, excludes issues whose terminal status
/// key isn't in the set (include-form filter). Issues that never
/// reached terminal continue to be excluded regardless.
pub fn compute_issues_time_in_status_breakdown(
    histories: &[IssueHistory],
    project_id: &ProjectId,
    project: &Project,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    status_keys: &[StatusKey],
) -> IssuesTimeInStatusBreakdownResponse {
    let mut total_per_status: HashMap<StatusKey, u64> = HashMap::new();
    let mut cohort_size: u64 = 0;

    for history in histories {
        if history.project_id() != project_id {
            continue;
        }
        let Some((_, terminal_at)) = history.first_terminal(project) else {
            continue;
        };
        if terminal_at < from || terminal_at >= to {
            continue;
        }
        if !issue_passes_status_filter(history, project, status_keys) {
            continue;
        }
        cohort_size += 1;

        // Walk versions in pairs; the duration each `(version_N).status`
        // contributes is `version_{N+1}.timestamp - version_N.timestamp`.
        // The last version contributes 0 — no successor to bound it.
        let versions = &history.versions;
        for window in versions.windows(2) {
            let curr = &window[0];
            let next = &window[1];
            let key = curr.item.status.clone();
            let delta = (next.timestamp - curr.timestamp).num_seconds().max(0) as u64;
            *total_per_status.entry(key).or_insert(0) += delta;
        }
    }

    let denom = cohort_size.max(1);

    let mut segments: Vec<TimeInStatusSegment> = Vec::with_capacity(project.statuses.len());
    for status in ordered_statuses(project) {
        let total = total_per_status.get(&status.key).copied().unwrap_or(0);
        let mean = if cohort_size == 0 { 0 } else { total / denom };
        segments.push(TimeInStatusSegment::new(
            status.key.clone(),
            status.label.clone(),
            status.color.clone(),
            mean,
        ));
    }

    IssuesTimeInStatusBreakdownResponse::new(project_id.clone(), segments, cohort_size)
}

/// Compute `issues/per_status_distribution` for a single project's
/// status set: per-status percentiles (median, p95) over every
/// `(issue, status)` dwell-segment that *ended* inside `[from, to)`.
/// An issue still sitting in a status when the window closes does not
/// contribute.
///
/// `status_keys`: when populated, only segments from issues that
/// reached a terminal status whose key is in the set contribute samples
/// (issues that never reached terminal are excluded). When empty, all
/// issues contribute their ended segments — including those that never
/// reached terminal — matching the unfiltered baseline.
pub fn compute_issues_per_status_distribution(
    histories: &[IssueHistory],
    project_id: &ProjectId,
    project: &Project,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    status_keys: &[StatusKey],
) -> IssuesPerStatusDistributionResponse {
    let mut samples_per_status: HashMap<StatusKey, Vec<u64>> = HashMap::new();
    for history in histories {
        if history.project_id() != project_id {
            continue;
        }
        if !issue_passes_status_filter(history, project, status_keys) {
            continue;
        }
        let versions = &history.versions;
        for window in versions.windows(2) {
            let curr = &window[0];
            let next = &window[1];
            // Segment ends at `next.timestamp`.
            if next.timestamp < from || next.timestamp >= to {
                continue;
            }
            let delta = (next.timestamp - curr.timestamp).num_seconds().max(0) as u64;
            samples_per_status
                .entry(curr.item.status.clone())
                .or_default()
                .push(delta);
        }
    }

    let mut out: Vec<PerStatusDistribution> = Vec::with_capacity(project.statuses.len());
    for status in ordered_statuses(project) {
        let mut samples = samples_per_status.remove(&status.key).unwrap_or_default();
        samples.sort_unstable();
        let sample_count = samples.len() as u64;
        let median = percentile(&samples, 0.5);
        let p95 = percentile(&samples, 0.95);
        out.push(PerStatusDistribution::new(
            status.key.clone(),
            status.label.clone(),
            status.color.clone(),
            median,
            p95,
            sample_count,
        ));
    }
    IssuesPerStatusDistributionResponse::new(project_id.clone(), out)
}

/// The project's status list ordered by the `position` field
/// (smaller-first), matching how the project itself renders the
/// statuses. Stable on equal positions (rare, but possible when the
/// project never reordered after seeding).
fn ordered_statuses(project: &Project) -> Vec<&StatusDefinition> {
    let mut statuses: Vec<&StatusDefinition> = project.statuses.iter().collect();
    statuses.sort_by(|a, b| {
        a.position
            .partial_cmp(&b.position)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    statuses
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issues::{
        Issue as DomainIssue, IssueType as DomainIssueType, SessionSettings,
    };
    use crate::domain::projects::{default_project_id, default_project_seed};
    use crate::domain::users::Username;
    use hydra_common::ActorRef as CommonActorRef;
    use hydra_common::RepoName;
    use hydra_common::api::v1::projects::StatusKey as ApiStatusKey;

    fn dt(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .expect("rfc3339 timestamp")
            .with_timezone(&Utc)
    }

    fn repo(name: &str) -> RepoName {
        let (org, repo) = name.split_once('/').expect("org/repo");
        RepoName::new(org, repo).expect("valid repo name")
    }

    fn skey(s: &str) -> ApiStatusKey {
        ApiStatusKey::try_new(s).expect("status key")
    }

    fn issue_in_default_project(status: &str, creator: &str) -> DomainIssue {
        DomainIssue::new(
            DomainIssueType::Task,
            "title".to_string(),
            "desc".to_string(),
            Username::from(creator),
            String::new(),
            skey(status),
            default_project_id(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        )
    }

    fn issue_versioned(
        item: DomainIssue,
        version: u64,
        timestamp: DateTime<Utc>,
    ) -> Versioned<DomainIssue> {
        Versioned {
            item,
            version,
            timestamp,
            actor: Some(CommonActorRef::test()),
            creation_time: timestamp,
        }
    }

    fn issue_history(versions: Vec<Versioned<DomainIssue>>) -> IssueHistory {
        IssueHistory::new(IssueId::new(), versions)
    }

    fn projects_map_default() -> HashMap<ProjectId, Project> {
        let mut map = HashMap::new();
        map.insert(default_project_id(), default_project_seed());
        map
    }

    // ----- cycle_time -----

    #[test]
    fn cycle_time_empty_cohort_yields_zero_count() {
        let resp = compute_issues_cycle_time(
            &[],
            &HashMap::new(),
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-13T00:00:00Z"),
            &[],
        );
        assert_eq!(resp.count, 0);
        assert!(resp.median_seconds.is_none());
        // Histogram is always emitted with zero-count bins.
        assert!(!resp.histogram.is_empty());
        for bin in &resp.histogram {
            assert_eq!(bin.count, 0);
        }
    }

    #[test]
    fn cycle_time_single_issue_collapses_median_and_p95() {
        let v1 = issue_versioned(
            issue_in_default_project("open", "alice"),
            1,
            dt("2026-05-10T00:00:00Z"),
        );
        let v2 = issue_versioned(
            issue_in_default_project("closed", "alice"),
            2,
            dt("2026-05-11T00:00:00Z"),
        );
        let histories = vec![issue_history(vec![v1, v2])];
        let projects = projects_map_default();
        let resp = compute_issues_cycle_time(
            &histories,
            &projects,
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-13T00:00:00Z"),
            &[],
        );
        assert_eq!(resp.count, 1);
        assert_eq!(resp.median_seconds, Some(86_400));
        assert_eq!(resp.p95_seconds, Some(86_400));
        let one_day_bin = resp
            .histogram
            .iter()
            .find(|b| b.bin_start_seconds == 86_400)
            .expect("1d bin");
        assert_eq!(one_day_bin.count, 1);
    }

    #[test]
    fn cycle_time_excludes_issues_still_in_nonterminal_status_at_window_close() {
        let v1 = issue_versioned(
            issue_in_default_project("open", "alice"),
            1,
            dt("2026-05-10T00:00:00Z"),
        );
        let v2 = issue_versioned(
            issue_in_default_project("in-progress", "alice"),
            2,
            dt("2026-05-11T00:00:00Z"),
        );
        let histories = vec![issue_history(vec![v1, v2])];
        let projects = projects_map_default();
        // Window closes before the issue is closed.
        let resp = compute_issues_cycle_time(
            &histories,
            &projects,
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-13T00:00:00Z"),
            &[],
        );
        assert_eq!(resp.count, 0);
    }

    #[test]
    fn cycle_time_terminal_outside_window_is_excluded() {
        let v1 = issue_versioned(
            issue_in_default_project("open", "alice"),
            1,
            dt("2026-05-01T00:00:00Z"),
        );
        let v2 = issue_versioned(
            issue_in_default_project("closed", "alice"),
            2,
            dt("2026-06-01T00:00:00Z"),
        );
        let histories = vec![issue_history(vec![v1, v2])];
        let projects = projects_map_default();
        let resp = compute_issues_cycle_time(
            &histories,
            &projects,
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-13T00:00:00Z"),
            &[],
        );
        assert_eq!(resp.count, 0);
    }

    #[test]
    fn cycle_time_status_keys_include_filter_drops_unmatched_terminals() {
        // Issue 1: closed (matches if status_keys=[closed]).
        let issue1 = vec![
            issue_versioned(
                issue_in_default_project("open", "alice"),
                1,
                dt("2026-05-10T00:00:00Z"),
            ),
            issue_versioned(
                issue_in_default_project("closed", "alice"),
                2,
                dt("2026-05-11T00:00:00Z"),
            ),
        ];
        // Issue 2: dropped (excluded if status_keys=[closed]).
        let issue2 = vec![
            issue_versioned(
                issue_in_default_project("open", "bob"),
                1,
                dt("2026-05-10T00:00:00Z"),
            ),
            issue_versioned(
                issue_in_default_project("dropped", "bob"),
                2,
                dt("2026-05-11T00:00:00Z"),
            ),
        ];
        let histories = vec![issue_history(issue1), issue_history(issue2)];
        let projects = projects_map_default();
        let resp = compute_issues_cycle_time(
            &histories,
            &projects,
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-13T00:00:00Z"),
            &[skey("closed")],
        );
        assert_eq!(resp.count, 1);
        // Without the filter, both count.
        let resp_no_filter = compute_issues_cycle_time(
            &histories,
            &projects,
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-13T00:00:00Z"),
            &[],
        );
        assert_eq!(resp_no_filter.count, 2);
    }

    // ----- over_time -----

    #[test]
    fn over_time_counts_creation_and_terminal_in_correct_buckets() {
        let from = dt("2026-05-10T00:00:00Z");
        let to = dt("2026-05-13T00:00:00Z");

        // Issue 1: created day 0, closed day 2.
        let i1 = vec![
            issue_versioned(
                issue_in_default_project("open", "alice"),
                1,
                dt("2026-05-10T08:00:00Z"),
            ),
            issue_versioned(
                issue_in_default_project("closed", "alice"),
                2,
                dt("2026-05-12T09:00:00Z"),
            ),
        ];
        // Issue 2: created day 1, still open.
        let i2 = vec![issue_versioned(
            issue_in_default_project("open", "bob"),
            1,
            dt("2026-05-11T08:00:00Z"),
        )];

        let histories = vec![issue_history(i1), issue_history(i2)];
        let projects = projects_map_default();
        let resp =
            compute_issues_over_time(&histories, &projects, from, to, BucketGranularity::Day, &[]);
        assert_eq!(resp.buckets.len(), 3);
        assert_eq!(resp.buckets[0].bucket_start, dt("2026-05-10T00:00:00Z"));
        assert_eq!(resp.buckets[0].created, 1);
        assert_eq!(resp.buckets[0].reached_terminal, 0);
        assert_eq!(resp.buckets[1].created, 1);
        assert_eq!(resp.buckets[1].reached_terminal, 0);
        assert_eq!(resp.buckets[2].created, 0);
        assert_eq!(resp.buckets[2].reached_terminal, 1);
    }

    // ----- time_in_status_breakdown -----

    #[test]
    fn time_in_status_breakdown_walks_versions_pairwise() {
        // Issue spends 1 day Open, 2 days In-progress, then Closed inside window.
        let v1 = issue_versioned(
            issue_in_default_project("open", "alice"),
            1,
            dt("2026-05-10T00:00:00Z"),
        );
        let v2 = issue_versioned(
            issue_in_default_project("in-progress", "alice"),
            2,
            dt("2026-05-11T00:00:00Z"),
        );
        let v3 = issue_versioned(
            issue_in_default_project("closed", "alice"),
            3,
            dt("2026-05-13T00:00:00Z"),
        );
        let histories = vec![issue_history(vec![v1, v2, v3])];
        let project = default_project_seed();
        let resp = compute_issues_time_in_status_breakdown(
            &histories,
            &default_project_id(),
            &project,
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-15T00:00:00Z"),
            &[],
        );
        assert_eq!(resp.issue_count, 1);
        let open_segment = resp
            .status_segments
            .iter()
            .find(|s| s.status_key == skey("open"))
            .expect("open segment");
        assert_eq!(open_segment.mean_seconds, 86_400); // 1 day
        let in_progress_segment = resp
            .status_segments
            .iter()
            .find(|s| s.status_key == skey("in-progress"))
            .expect("in-progress segment");
        assert_eq!(in_progress_segment.mean_seconds, 2 * 86_400); // 2 days
        let closed_segment = resp
            .status_segments
            .iter()
            .find(|s| s.status_key == skey("closed"))
            .expect("closed segment");
        assert_eq!(closed_segment.mean_seconds, 0); // terminal contributes 0
    }

    #[test]
    fn time_in_status_breakdown_accumulates_revisited_status() {
        // Issue bounces: open(1d) -> in-progress(2d) -> open(1d) -> in-progress(3d) -> closed.
        // Total open dwell: 2 days. Total in-progress dwell: 5 days. Closed: 0.
        let v1 = issue_versioned(
            issue_in_default_project("open", "alice"),
            1,
            dt("2026-05-10T00:00:00Z"),
        );
        let v2 = issue_versioned(
            issue_in_default_project("in-progress", "alice"),
            2,
            dt("2026-05-11T00:00:00Z"),
        );
        let v3 = issue_versioned(
            issue_in_default_project("open", "alice"),
            3,
            dt("2026-05-13T00:00:00Z"),
        );
        let v4 = issue_versioned(
            issue_in_default_project("in-progress", "alice"),
            4,
            dt("2026-05-14T00:00:00Z"),
        );
        let v5 = issue_versioned(
            issue_in_default_project("closed", "alice"),
            5,
            dt("2026-05-17T00:00:00Z"),
        );
        let histories = vec![issue_history(vec![v1, v2, v3, v4, v5])];
        let project = default_project_seed();
        let resp = compute_issues_time_in_status_breakdown(
            &histories,
            &default_project_id(),
            &project,
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-20T00:00:00Z"),
            &[],
        );
        assert_eq!(resp.issue_count, 1);
        let open_seg = resp
            .status_segments
            .iter()
            .find(|s| s.status_key == skey("open"))
            .expect("open");
        assert_eq!(open_seg.mean_seconds, 2 * 86_400);
        let in_progress_seg = resp
            .status_segments
            .iter()
            .find(|s| s.status_key == skey("in-progress"))
            .expect("in-progress");
        assert_eq!(in_progress_seg.mean_seconds, 5 * 86_400);
    }

    #[test]
    fn time_in_status_breakdown_excludes_issues_outside_cohort() {
        // One issue closed in window, one still open.
        let closed = vec![
            issue_versioned(
                issue_in_default_project("open", "alice"),
                1,
                dt("2026-05-10T00:00:00Z"),
            ),
            issue_versioned(
                issue_in_default_project("closed", "alice"),
                2,
                dt("2026-05-11T00:00:00Z"),
            ),
        ];
        let still_open = vec![issue_versioned(
            issue_in_default_project("in-progress", "bob"),
            1,
            dt("2026-05-10T00:00:00Z"),
        )];
        let histories = vec![issue_history(closed), issue_history(still_open)];
        let project = default_project_seed();
        let resp = compute_issues_time_in_status_breakdown(
            &histories,
            &default_project_id(),
            &project,
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-13T00:00:00Z"),
            &[],
        );
        assert_eq!(resp.issue_count, 1);
    }

    // ----- per_status_distribution -----

    #[test]
    fn per_status_distribution_collects_only_ended_segments() {
        // Issue 1: open for 1 day then in-progress for 2 days (segment
        // ends 05-13), then closed at 05-13. Both segments end inside
        // [05-10, 05-15).
        let v1 = issue_versioned(
            issue_in_default_project("open", "alice"),
            1,
            dt("2026-05-10T00:00:00Z"),
        );
        let v2 = issue_versioned(
            issue_in_default_project("in-progress", "alice"),
            2,
            dt("2026-05-11T00:00:00Z"),
        );
        let v3 = issue_versioned(
            issue_in_default_project("closed", "alice"),
            3,
            dt("2026-05-13T00:00:00Z"),
        );
        // Issue 2: still in-progress (no ending segment) — excluded.
        let still = vec![issue_versioned(
            issue_in_default_project("in-progress", "bob"),
            1,
            dt("2026-05-10T00:00:00Z"),
        )];
        let histories = vec![issue_history(vec![v1, v2, v3]), issue_history(still)];
        let project = default_project_seed();
        let resp = compute_issues_per_status_distribution(
            &histories,
            &default_project_id(),
            &project,
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-15T00:00:00Z"),
            &[],
        );
        let open_dist = resp
            .statuses
            .iter()
            .find(|s| s.status_key == skey("open"))
            .expect("open");
        assert_eq!(open_dist.sample_count, 1);
        assert_eq!(open_dist.median_seconds, Some(86_400));
        let in_progress_dist = resp
            .statuses
            .iter()
            .find(|s| s.status_key == skey("in-progress"))
            .expect("in-progress");
        assert_eq!(in_progress_dist.sample_count, 1);
        assert_eq!(in_progress_dist.median_seconds, Some(2 * 86_400));
        // Closed is never exited in the cohort, so no samples.
        let closed_dist = resp
            .statuses
            .iter()
            .find(|s| s.status_key == skey("closed"))
            .expect("closed");
        assert_eq!(closed_dist.sample_count, 0);
        assert!(closed_dist.median_seconds.is_none());
    }

    #[test]
    fn over_time_status_keys_include_filter_drops_unmatched_terminal_increment() {
        let from = dt("2026-05-10T00:00:00Z");
        let to = dt("2026-05-13T00:00:00Z");
        // Issue 1: closed in window (matches if status_keys=[closed]).
        let i1 = vec![
            issue_versioned(
                issue_in_default_project("open", "alice"),
                1,
                dt("2026-05-10T08:00:00Z"),
            ),
            issue_versioned(
                issue_in_default_project("closed", "alice"),
                2,
                dt("2026-05-11T08:00:00Z"),
            ),
        ];
        // Issue 2: dropped in window (excluded if status_keys=[closed]).
        let i2 = vec![
            issue_versioned(
                issue_in_default_project("open", "bob"),
                1,
                dt("2026-05-10T08:00:00Z"),
            ),
            issue_versioned(
                issue_in_default_project("dropped", "bob"),
                2,
                dt("2026-05-12T08:00:00Z"),
            ),
        ];
        let histories = vec![issue_history(i1), issue_history(i2)];
        let projects = projects_map_default();
        let resp = compute_issues_over_time(
            &histories,
            &projects,
            from,
            to,
            BucketGranularity::Day,
            &[skey("closed")],
        );
        // Both issues created within window — created counts unfiltered.
        let total_created: u64 = resp.buckets.iter().map(|b| b.created).sum();
        assert_eq!(total_created, 2);
        // Only the closed-terminal issue contributes reached_terminal.
        let total_terminal: u64 = resp.buckets.iter().map(|b| b.reached_terminal).sum();
        assert_eq!(total_terminal, 1);
        // The increment lands in the day-1 bucket (the closed flip).
        assert_eq!(resp.buckets[1].reached_terminal, 1);
        assert_eq!(resp.buckets[2].reached_terminal, 0);
        // Without the filter, both terminals count.
        let resp_no_filter =
            compute_issues_over_time(&histories, &projects, from, to, BucketGranularity::Day, &[]);
        let total_terminal_no_filter: u64 = resp_no_filter
            .buckets
            .iter()
            .map(|b| b.reached_terminal)
            .sum();
        assert_eq!(total_terminal_no_filter, 2);
    }

    #[test]
    fn time_in_status_breakdown_status_keys_include_filter_drops_unmatched_cohort() {
        // Issue 1: closed (matches if status_keys=[closed]).
        let i1 = vec![
            issue_versioned(
                issue_in_default_project("open", "alice"),
                1,
                dt("2026-05-10T00:00:00Z"),
            ),
            issue_versioned(
                issue_in_default_project("closed", "alice"),
                2,
                dt("2026-05-11T00:00:00Z"),
            ),
        ];
        // Issue 2: dropped (excluded if status_keys=[closed]).
        let i2 = vec![
            issue_versioned(
                issue_in_default_project("open", "bob"),
                1,
                dt("2026-05-10T00:00:00Z"),
            ),
            issue_versioned(
                issue_in_default_project("dropped", "bob"),
                2,
                dt("2026-05-11T00:00:00Z"),
            ),
        ];
        let histories = vec![issue_history(i1), issue_history(i2)];
        let project = default_project_seed();
        let resp = compute_issues_time_in_status_breakdown(
            &histories,
            &default_project_id(),
            &project,
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-13T00:00:00Z"),
            &[skey("closed")],
        );
        assert_eq!(resp.issue_count, 1);
        // Without the filter, both issues are in the cohort.
        let resp_no_filter = compute_issues_time_in_status_breakdown(
            &histories,
            &default_project_id(),
            &project,
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-13T00:00:00Z"),
            &[],
        );
        assert_eq!(resp_no_filter.issue_count, 2);
    }

    #[test]
    fn per_status_distribution_status_keys_include_filter_drops_unmatched_terminals() {
        // Issue 1: open for 1d, then closed in window.
        let i1 = vec![
            issue_versioned(
                issue_in_default_project("open", "alice"),
                1,
                dt("2026-05-10T00:00:00Z"),
            ),
            issue_versioned(
                issue_in_default_project("closed", "alice"),
                2,
                dt("2026-05-11T00:00:00Z"),
            ),
        ];
        // Issue 2: open for 2d, then dropped in window — excluded when
        // status_keys=[closed]. Its `open` segment would otherwise show.
        let i2 = vec![
            issue_versioned(
                issue_in_default_project("open", "bob"),
                1,
                dt("2026-05-10T00:00:00Z"),
            ),
            issue_versioned(
                issue_in_default_project("dropped", "bob"),
                2,
                dt("2026-05-12T00:00:00Z"),
            ),
        ];
        let histories = vec![issue_history(i1), issue_history(i2)];
        let project = default_project_seed();
        let resp = compute_issues_per_status_distribution(
            &histories,
            &default_project_id(),
            &project,
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-13T00:00:00Z"),
            &[skey("closed")],
        );
        let open_dist = resp
            .statuses
            .iter()
            .find(|s| s.status_key == skey("open"))
            .expect("open");
        // Only the closed-terminal issue's 1-day open segment counts.
        assert_eq!(open_dist.sample_count, 1);
        assert_eq!(open_dist.median_seconds, Some(86_400));
        // Without the filter, both issues' open segments contribute.
        let resp_no_filter = compute_issues_per_status_distribution(
            &histories,
            &default_project_id(),
            &project,
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-13T00:00:00Z"),
            &[],
        );
        let open_dist_no_filter = resp_no_filter
            .statuses
            .iter()
            .find(|s| s.status_key == skey("open"))
            .expect("open");
        assert_eq!(open_dist_no_filter.sample_count, 2);
    }

    // ----- fetch / status-set evolution -----

    #[tokio::test]
    async fn fetch_issue_histories_excludes_deleted() {
        use crate::test_utils::test_state_handles;
        let handles = test_state_handles();
        let store = handles.store.clone();
        let actor = CommonActorRef::test();
        let normal = issue_in_default_project("open", "alice");
        let (normal_id, _) = store.add_issue(normal, &actor).await.expect("add normal");
        let to_delete = issue_in_default_project("open", "alice");
        let (delete_id, _) = store
            .add_issue(to_delete, &actor)
            .await
            .expect("add to_delete");
        store
            .delete_issue(&delete_id, &actor)
            .await
            .expect("delete");

        let query =
            IssuesThroughputQuery::new(dt("2026-05-10T00:00:00Z"), dt("2026-05-13T00:00:00Z"));
        let histories = fetch_issue_histories(store.as_ref(), &query)
            .await
            .expect("fetch");
        let ids: Vec<_> = histories.iter().map(|h| h.issue_id.clone()).collect();
        assert_eq!(ids, vec![normal_id]);
    }

    fn issue_in_default_project_typed(
        issue_type: DomainIssueType,
        status: &str,
        creator: &str,
    ) -> DomainIssue {
        DomainIssue::new(
            issue_type,
            "title".to_string(),
            "desc".to_string(),
            Username::from(creator),
            String::new(),
            skey(status),
            default_project_id(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        )
    }

    #[tokio::test]
    async fn fetch_issue_histories_issue_types_returns_union() {
        use crate::test_utils::test_state_handles;
        use hydra_common::api::v1::issues::IssueType as ApiIssueType;
        let handles = test_state_handles();
        let store = handles.store.clone();
        let actor = CommonActorRef::test();

        let feature = issue_in_default_project_typed(DomainIssueType::Feature, "open", "alice");
        let (feature_id, _) = store.add_issue(feature, &actor).await.expect("add feature");
        let bug = issue_in_default_project_typed(DomainIssueType::Bug, "open", "alice");
        let (bug_id, _) = store.add_issue(bug, &actor).await.expect("add bug");
        let task = issue_in_default_project_typed(DomainIssueType::Task, "open", "alice");
        let (_, _) = store.add_issue(task, &actor).await.expect("add task");

        let mut query =
            IssuesThroughputQuery::new(dt("2026-05-10T00:00:00Z"), dt("2026-05-13T00:00:00Z"));
        query.issue_types = vec![ApiIssueType::Feature, ApiIssueType::Bug];
        let histories = fetch_issue_histories(store.as_ref(), &query)
            .await
            .expect("fetch");
        let mut ids: Vec<_> = histories.iter().map(|h| h.issue_id.clone()).collect();
        ids.sort();
        let mut expected = vec![feature_id, bug_id];
        expected.sort();
        assert_eq!(ids, expected);
    }

    #[tokio::test]
    async fn fetch_issue_histories_issue_type_singular_fallback() {
        use crate::test_utils::test_state_handles;
        use hydra_common::api::v1::issues::IssueType as ApiIssueType;
        let handles = test_state_handles();
        let store = handles.store.clone();
        let actor = CommonActorRef::test();

        let feature = issue_in_default_project_typed(DomainIssueType::Feature, "open", "alice");
        let (feature_id, _) = store.add_issue(feature, &actor).await.expect("add feature");
        let bug = issue_in_default_project_typed(DomainIssueType::Bug, "open", "alice");
        let (_, _) = store.add_issue(bug, &actor).await.expect("add bug");

        let mut query =
            IssuesThroughputQuery::new(dt("2026-05-10T00:00:00Z"), dt("2026-05-13T00:00:00Z"));
        query.issue_type = Some(ApiIssueType::Feature);
        // issue_types empty → singular field applies.
        let histories = fetch_issue_histories(store.as_ref(), &query)
            .await
            .expect("fetch");
        let ids: Vec<_> = histories.iter().map(|h| h.issue_id.clone()).collect();
        assert_eq!(ids, vec![feature_id]);
    }

    #[tokio::test]
    async fn fetch_issue_histories_no_type_filter_returns_all() {
        use crate::test_utils::test_state_handles;
        let handles = test_state_handles();
        let store = handles.store.clone();
        let actor = CommonActorRef::test();

        let feature = issue_in_default_project_typed(DomainIssueType::Feature, "open", "alice");
        store.add_issue(feature, &actor).await.expect("add feature");
        let bug = issue_in_default_project_typed(DomainIssueType::Bug, "open", "alice");
        store.add_issue(bug, &actor).await.expect("add bug");
        let task = issue_in_default_project_typed(DomainIssueType::Task, "open", "alice");
        store.add_issue(task, &actor).await.expect("add task");

        let query =
            IssuesThroughputQuery::new(dt("2026-05-10T00:00:00Z"), dt("2026-05-13T00:00:00Z"));
        // Both issue_type and issue_types unset.
        let histories = fetch_issue_histories(store.as_ref(), &query)
            .await
            .expect("fetch");
        assert_eq!(histories.len(), 3);
    }

    #[tokio::test]
    async fn fetch_issue_histories_issue_types_supersedes_singular() {
        use crate::test_utils::test_state_handles;
        use hydra_common::api::v1::issues::IssueType as ApiIssueType;
        let handles = test_state_handles();
        let store = handles.store.clone();
        let actor = CommonActorRef::test();

        let feature = issue_in_default_project_typed(DomainIssueType::Feature, "open", "alice");
        let (feature_id, _) = store.add_issue(feature, &actor).await.expect("add feature");
        let bug = issue_in_default_project_typed(DomainIssueType::Bug, "open", "alice");
        let (_, _) = store.add_issue(bug, &actor).await.expect("add bug");

        let mut query =
            IssuesThroughputQuery::new(dt("2026-05-10T00:00:00Z"), dt("2026-05-13T00:00:00Z"));
        // Non-empty issue_types wins; singular issue_type is ignored.
        query.issue_types = vec![ApiIssueType::Feature];
        query.issue_type = Some(ApiIssueType::Bug);
        let histories = fetch_issue_histories(store.as_ref(), &query)
            .await
            .expect("fetch");
        let ids: Vec<_> = histories.iter().map(|h| h.issue_id.clone()).collect();
        assert_eq!(ids, vec![feature_id]);
    }

    #[tokio::test]
    async fn fetch_issue_histories_filters_by_creator_and_repo() {
        use crate::test_utils::test_state_handles;
        let handles = test_state_handles();
        let store = handles.store.clone();
        let actor = CommonActorRef::test();

        let mut alice_a = issue_in_default_project("open", "alice");
        alice_a.session_settings = SessionSettings::default();
        alice_a.session_settings.repo_name = Some(repo("dourolabs/hydra"));
        let (alice_a_id, _) = store.add_issue(alice_a, &actor).await.expect("alice_a");

        let mut bob_a = issue_in_default_project("open", "bob");
        bob_a.session_settings.repo_name = Some(repo("dourolabs/hydra"));
        let (_, _) = store.add_issue(bob_a, &actor).await.expect("bob_a");

        let mut alice_b = issue_in_default_project("open", "alice");
        alice_b.session_settings.repo_name = Some(repo("other/repo"));
        let (_, _) = store.add_issue(alice_b, &actor).await.expect("alice_b");

        let mut query =
            IssuesThroughputQuery::new(dt("2026-05-10T00:00:00Z"), dt("2026-05-13T00:00:00Z"));
        query.creator = Some("alice".to_string());
        query.repo_name = Some("dourolabs/hydra".to_string());
        let histories = fetch_issue_histories(store.as_ref(), &query)
            .await
            .expect("fetch");
        let ids: Vec<_> = histories.iter().map(|h| h.issue_id.clone()).collect();
        assert_eq!(ids, vec![alice_a_id]);
    }

    #[test]
    fn cycle_time_ignores_issue_whose_status_key_is_missing_from_project() {
        // If a status key on an issue version isn't declared in the
        // current project (e.g. the user deleted the status mid-history
        // and the rewrite did not migrate this row), the analytics walk
        // skips that version's terminal-check. The issue effectively
        // never reaches a terminal status from the analytics' POV.
        let v1 = issue_versioned(
            issue_in_default_project("open", "alice"),
            1,
            dt("2026-05-10T00:00:00Z"),
        );
        // Forge a version with a status key that's not in the project.
        let mut bogus = issue_in_default_project("open", "alice");
        bogus.status = skey("ghost");
        let v2 = issue_versioned(bogus, 2, dt("2026-05-11T00:00:00Z"));
        let histories = vec![issue_history(vec![v1, v2])];
        let projects = projects_map_default();
        let resp = compute_issues_cycle_time(
            &histories,
            &projects,
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-13T00:00:00Z"),
            &[],
        );
        assert_eq!(resp.count, 0);
    }

    #[tokio::test]
    async fn cycle_time_after_status_rename_keeps_terminal_flip() {
        // When SWE renames a status mid-history, `get_issue_versions`
        // returns versions with the *current* key (translate_issue_status
        // rewrites them via the (project_id, sequence) FK). So the
        // analytics walk just sees the current key on every version
        // and the terminal flip still resolves cleanly.
        use crate::test_utils::test_state_handles;
        let handles = test_state_handles();
        let store = handles.store.clone();
        let actor = CommonActorRef::test();

        let initial = issue_in_default_project("open", "alice");
        let (issue_id, _) = store.add_issue(initial, &actor).await.expect("add");
        let mut closed = issue_in_default_project("closed", "alice");
        closed.dependencies = vec![];
        store
            .update_issue(&issue_id, closed, &actor)
            .await
            .expect("close");

        let query =
            IssuesThroughputQuery::new(dt("2020-01-01T00:00:00Z"), dt("2030-01-01T00:00:00Z"));
        let histories = fetch_issue_histories(store.as_ref(), &query)
            .await
            .expect("fetch");
        assert_eq!(histories.len(), 1);
        let projects = resolve_projects_for_histories(store.as_ref(), &histories)
            .await
            .expect("resolve");
        let resp = compute_issues_cycle_time(
            &histories,
            &projects,
            dt("2020-01-01T00:00:00Z"),
            dt("2030-01-01T00:00:00Z"),
            &[],
        );
        assert_eq!(resp.count, 1);
    }
}
