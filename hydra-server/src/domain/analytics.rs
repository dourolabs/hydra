//! In-process analytics aggregation over patch and issue version
//! histories.
//!
//! Backed by the existing `list_patches` / `get_patch_versions` /
//! `list_issues` / `get_issue_versions` store primitives — no new
//! `Store`-trait methods, no materialized tables. The aggregation walks
//! each entity's full version history in memory. Past production scale
//! this will need a push-down rewrite, but it buys us a complete feature
//! without a parallel store surface to maintain in lockstep.
//!
//! ## "Terminal" — issues
//!
//! A status is **terminal** iff `unblocks_parents = TRUE` on its
//! [`StatusDefinition`] — same criterion as
//! `policy/restrictions/issue_lifecycle.rs::is_terminal` (line 153).
//! `closed`, `dropped`, and `failed` are all terminal under this
//! definition; clients that want to exclude the cancellation lanes can
//! pass `status_keys=closed` on the query.

use crate::app::projects::{ResolveStatusError, project_cached};
use crate::domain::issues::Issue;
use crate::domain::patches::{Patch, PatchStatus};
use crate::store::{ReadOnlyStore, StoreError};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use hydra_common::api::v1::analytics::{
    BucketGranularity, IssueOverTimeBucket, IssuesCycleTimeResponse, IssuesOverTimeResponse,
    IssuesPerStatusDistributionResponse, IssuesThroughputQuery,
    IssuesTimeInStatusBreakdownResponse, PatchInFlightBucket, PatchOverTimeBucket,
    PatchesInFlightOverTimeResponse, PatchesOverTimeResponse, PatchesTerminalMixResponse,
    PatchesThroughputQuery, PatchesTimeToMergeResponse, PerStatusDistribution, TimeInStatusSegment,
    TimeToMergeBin,
};
use hydra_common::api::v1::issues::SearchIssuesQuery;
use hydra_common::api::v1::patches::SearchPatchesQuery;
use hydra_common::api::v1::projects::{Project, StatusDefinition, StatusKey};
use hydra_common::principal::Principal;
use hydra_common::{IssueId, PatchId, ProjectId, Versioned};
use std::collections::HashMap;
use std::str::FromStr;

/// One patch and its full ascending-version-order history. Aggregation
/// inputs are intentionally simple structs so unit tests can pass
/// hand-rolled fixtures without touching a Store.
#[derive(Debug, Clone)]
pub struct PatchHistory {
    pub patch_id: PatchId,
    pub versions: Vec<Versioned<Patch>>,
}

impl PatchHistory {
    pub fn new(patch_id: PatchId, versions: Vec<Versioned<Patch>>) -> Self {
        Self { patch_id, versions }
    }

    /// Creation timestamp (first version's `timestamp`).
    fn created_at(&self) -> DateTime<Utc> {
        self.versions
            .first()
            .expect("patch history must contain at least one version")
            .timestamp
    }

    /// First version whose stored status is `Merged`, regardless of
    /// what later versions do. Matches the spec's "Once you've found
    /// that flip, subsequent versions don't move the timestamp." rule.
    fn merged_at(&self) -> Option<DateTime<Utc>> {
        self.versions
            .iter()
            .find(|v| v.item.status == PatchStatus::Merged)
            .map(|v| v.timestamp)
    }

    /// First terminal-state version, or `None` if the patch never
    /// reached a terminal state. The "flip to terminal" timestamp used
    /// by the `terminal_mix` endpoint.
    fn first_terminal(&self) -> Option<(PatchStatus, DateTime<Utc>)> {
        self.versions
            .iter()
            .find(|v| matches!(v.item.status, PatchStatus::Merged | PatchStatus::Closed))
            .map(|v| (v.item.status, v.timestamp))
    }

    /// Status at the given moment: the latest version with
    /// `timestamp <= at`. `None` when the patch did not exist yet at
    /// `at` (i.e. all versions are strictly later).
    fn status_at(&self, at: DateTime<Utc>) -> Option<PatchStatus> {
        self.versions
            .iter()
            .rev()
            .find(|v| v.timestamp <= at)
            .map(|v| v.item.status)
    }
}

/// Fetch the patches matching the throughput-query filters and their
/// full version histories. Deleted patches and `is_automatic_backup`
/// patches are excluded — both checks run against the latest stored
/// version. The `from`/`to`/`bucket` fields on `query` are not used
/// here; the pure aggregators apply the time window.
pub async fn fetch_patch_histories(
    store: &dyn ReadOnlyStore,
    query: &PatchesThroughputQuery,
) -> Result<Vec<PatchHistory>, StoreError> {
    // `list_patches` already filters out `deleted=true` at latest by
    // default. `is_automatic_backup` isn't a list-level filter, so we
    // drop those in the loop below.
    let mut search = SearchPatchesQuery::default();
    search.repo_name = query.repo_name.clone();
    search.creator = query.creator.clone();
    search.status = query.status.clone();

    let patches = store.list_patches(&search).await?;
    let mut histories = Vec::with_capacity(patches.len());
    for (patch_id, latest) in patches {
        if latest.item.is_automatic_backup {
            continue;
        }
        let versions = store.get_patch_versions(&patch_id).await?;
        if versions.is_empty() {
            continue;
        }
        histories.push(PatchHistory::new(patch_id, versions));
    }
    Ok(histories)
}

/// Snap a timestamp down to the start of its enclosing bucket.
///
/// - `Day` buckets are UTC midnight of the same date.
/// - `Week` buckets are Monday UTC 00:00 (ISO weeks).
fn floor_to_bucket(t: DateTime<Utc>, bucket: BucketGranularity) -> DateTime<Utc> {
    let date = t.date_naive();
    match bucket {
        BucketGranularity::Day => to_utc_midnight(date),
        BucketGranularity::Week => {
            let days_since_monday = date.weekday().num_days_from_monday();
            let monday = date - Duration::days(days_since_monday as i64);
            to_utc_midnight(monday)
        }
        // `BucketGranularity` is `#[non_exhaustive]` for forward-compat on
        // the wire, but the server only knows day/week today; the
        // deserializer rejects any other value before we get here.
        _ => unreachable!("unsupported BucketGranularity variant"),
    }
}

fn to_utc_midnight(date: NaiveDate) -> DateTime<Utc> {
    let naive = NaiveDateTime::new(
        date,
        NaiveTime::from_hms_opt(0, 0, 0).expect("midnight is a valid time"),
    );
    DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc)
}

fn step(bucket: BucketGranularity) -> Duration {
    match bucket {
        BucketGranularity::Day => Duration::days(1),
        BucketGranularity::Week => Duration::days(7),
        _ => unreachable!("unsupported BucketGranularity variant"),
    }
}

/// All bucket starts that intersect `[from, to)`, in ascending order.
/// The first bucket may start before `from` (we snap `from` down to
/// the bucket boundary). The series terminates at the latest bucket
/// whose start is strictly less than `to`.
fn bucket_starts(
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    bucket: BucketGranularity,
) -> Vec<DateTime<Utc>> {
    if from >= to {
        return Vec::new();
    }
    let mut current = floor_to_bucket(from, bucket);
    let step = step(bucket);
    let mut out = Vec::new();
    while current < to {
        out.push(current);
        current += step;
    }
    out
}

/// Compute `patches/over_time` series: per bucket, the count of patches
/// whose creation timestamp lands in the bucket and the count of
/// patches whose first transition-to-merged timestamp lands in the
/// bucket. Buckets with zero hits are kept so the frontend gets a
/// dense series.
pub fn compute_patches_over_time(
    histories: &[PatchHistory],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    bucket: BucketGranularity,
) -> PatchesOverTimeResponse {
    let starts = bucket_starts(from, to, bucket);
    if starts.is_empty() {
        return PatchesOverTimeResponse::new(Vec::new());
    }
    let step = step(bucket);

    let mut buckets: Vec<PatchOverTimeBucket> = starts
        .iter()
        .map(|s| PatchOverTimeBucket::new(*s, 0, 0))
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
        if let Some(merged_at) = history.merged_at() {
            if let Some(idx) = bucket_for(merged_at) {
                buckets[idx].merged += 1;
            }
        }
    }

    PatchesOverTimeResponse::new(buckets)
}

/// Compute `patches/terminal_mix`: count patches by the terminal state
/// they first flipped to, provided that flip falls inside `[from, to)`.
pub fn compute_patches_terminal_mix(
    histories: &[PatchHistory],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> PatchesTerminalMixResponse {
    let mut merged = 0u64;
    let mut closed = 0u64;
    for history in histories {
        let Some((status, flip_at)) = history.first_terminal() else {
            continue;
        };
        if flip_at < from || flip_at >= to {
            continue;
        }
        match status {
            PatchStatus::Merged => merged += 1,
            PatchStatus::Closed => closed += 1,
            // first_terminal only returns Merged/Closed.
            PatchStatus::Open | PatchStatus::ChangesRequested => {}
        }
    }
    PatchesTerminalMixResponse::new(merged, closed)
}

/// Fixed histogram bin edges (in seconds) shared by the patches
/// `time_to_merge` and issues `cycle_time` endpoints. The final bin has
/// no upper bound; everything `>= last edge` lands in it. Documented on
/// each response type.
const DURATION_BIN_EDGES: &[u64] = &[
    0, 3_600,     // 1h
    14_400,    // 4h
    86_400,    // 1d
    259_200,   // 3d
    604_800,   // 7d
    1_209_600, // 14d
    2_592_000, // 30d
];

fn empty_duration_histogram() -> Vec<TimeToMergeBin> {
    let mut bins = Vec::with_capacity(DURATION_BIN_EDGES.len());
    for window in DURATION_BIN_EDGES.windows(2) {
        bins.push(TimeToMergeBin::new(window[0], Some(window[1]), 0));
    }
    let last = *DURATION_BIN_EDGES
        .last()
        .expect("bin edge list is non-empty");
    bins.push(TimeToMergeBin::new(last, None, 0));
    bins
}

fn bin_index_for(seconds: u64) -> usize {
    // The final open-ended bin owns anything >= last edge.
    for (i, window) in DURATION_BIN_EDGES.windows(2).enumerate() {
        if seconds < window[1] {
            return i;
        }
    }
    DURATION_BIN_EDGES.len() - 1
}

fn percentile(sorted: &[u64], p: f64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    if sorted.len() == 1 {
        return Some(sorted[0]);
    }
    // Nearest-rank percentile: ceil(p * n) - 1, clamped.
    let n = sorted.len() as f64;
    let rank = (p * n).ceil() as usize;
    let idx = rank.saturating_sub(1).min(sorted.len() - 1);
    Some(sorted[idx])
}

/// Compute `patches/time_to_merge`: histogram of merge latency over
/// patches whose `merged_at` falls inside `[from, to)`. `merged_at` is
/// the timestamp of the first version with status `Merged`.
pub fn compute_patches_time_to_merge(
    histories: &[PatchHistory],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> PatchesTimeToMergeResponse {
    let mut deltas: Vec<u64> = Vec::new();
    let mut histogram = empty_duration_histogram();
    for history in histories {
        let Some(merged_at) = history.merged_at() else {
            continue;
        };
        if merged_at < from || merged_at >= to {
            continue;
        }
        let created = history.created_at();
        // A merged_at strictly before created would be a corrupt
        // history; clamp to 0 rather than panic so analytics stays
        // best-effort.
        let delta = (merged_at - created).num_seconds().max(0) as u64;
        deltas.push(delta);
        let idx = bin_index_for(delta);
        histogram[idx].count += 1;
    }

    deltas.sort_unstable();
    let count = deltas.len() as u64;
    let median = percentile(&deltas, 0.5);
    let p95 = percentile(&deltas, 0.95);
    PatchesTimeToMergeResponse::new(median, p95, count, histogram)
}

/// Compute `patches/in_flight_over_time`: snapshot count of patches in
/// `Open` or `ChangesRequested` at each bucket boundary inside
/// `[from, to)`.
pub fn compute_patches_in_flight_over_time(
    histories: &[PatchHistory],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    bucket: BucketGranularity,
) -> PatchesInFlightOverTimeResponse {
    let starts = bucket_starts(from, to, bucket);
    let buckets = starts
        .into_iter()
        .map(|start| {
            let in_flight = histories
                .iter()
                .filter(|h| {
                    matches!(
                        h.status_at(start),
                        Some(PatchStatus::Open) | Some(PatchStatus::ChangesRequested)
                    )
                })
                .count() as u64;
            PatchInFlightBucket::new(start, in_flight)
        })
        .collect();
    PatchesInFlightOverTimeResponse::new(buckets)
}

// ====================== Issue analytics ======================

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
/// `issue_type` / `issue_types`. When the plural set is non-empty it
/// acts as an *include* set and the singular field is ignored; an empty
/// plural set falls back to the legacy singular match. The store-side
/// filters (`creator`, `project_id`) are pushed down through
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
        let any_match = query.issue_types.iter().any(|t| {
            let domain_type: crate::domain::issues::IssueType = (*t).into();
            issue.issue_type == domain_type
        });
        if !any_match {
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
    use crate::domain::patches::Patch;
    use crate::domain::users::Username;
    use hydra_common::ActorRef as CommonActorRef;
    use hydra_common::RepoName;

    fn dt(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .expect("rfc3339 timestamp")
            .with_timezone(&Utc)
    }

    fn repo(name: &str) -> RepoName {
        let (org, repo) = name.split_once('/').expect("org/repo");
        RepoName::new(org, repo).expect("valid repo name")
    }

    fn patch_with_status(
        status: PatchStatus,
        creator: &str,
        repo_name: RepoName,
        is_automatic_backup: bool,
    ) -> Patch {
        Patch::new(
            "title".to_string(),
            "desc".to_string(),
            "diff".to_string(),
            status,
            is_automatic_backup,
            Username::from(creator),
            Vec::new(),
            repo_name,
            None,
            None,
            None,
            None,
        )
    }

    /// Build a Versioned<Patch> with a controlled timestamp. The
    /// per-version `creation_time` field is kept consistent across the
    /// history by the caller; tests rarely care about it because the
    /// aggregator reads `versions[0].timestamp` for created_at.
    fn versioned(patch: Patch, version: u64, timestamp: DateTime<Utc>) -> Versioned<Patch> {
        Versioned {
            item: patch,
            version,
            timestamp,
            actor: Some(CommonActorRef::test()),
            creation_time: timestamp,
        }
    }

    fn history(versions: Vec<Versioned<Patch>>) -> PatchHistory {
        PatchHistory::new(PatchId::new(), versions)
    }

    // ----- bucket helpers -----

    #[test]
    fn day_bucket_floors_to_midnight() {
        let t = dt("2026-05-10T15:30:00Z");
        let floored = floor_to_bucket(t, BucketGranularity::Day);
        assert_eq!(floored, dt("2026-05-10T00:00:00Z"));
    }

    #[test]
    fn week_bucket_aligns_to_monday_utc() {
        // 2026-05-10 is a Sunday; Monday before it is 2026-05-04.
        let t = dt("2026-05-10T15:30:00Z");
        let floored = floor_to_bucket(t, BucketGranularity::Week);
        assert_eq!(floored, dt("2026-05-04T00:00:00Z"));
        // 2026-05-04 is itself Monday — should snap to itself.
        let monday = dt("2026-05-04T08:00:00Z");
        assert_eq!(
            floor_to_bucket(monday, BucketGranularity::Week),
            dt("2026-05-04T00:00:00Z")
        );
    }

    #[test]
    fn bucket_starts_empty_when_window_is_inverted() {
        let starts = bucket_starts(
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-09T00:00:00Z"),
            BucketGranularity::Day,
        );
        assert!(starts.is_empty());
    }

    #[test]
    fn bucket_starts_emits_dense_day_series() {
        let starts = bucket_starts(
            dt("2026-05-10T12:00:00Z"),
            dt("2026-05-13T00:00:00Z"),
            BucketGranularity::Day,
        );
        assert_eq!(
            starts,
            vec![
                dt("2026-05-10T00:00:00Z"),
                dt("2026-05-11T00:00:00Z"),
                dt("2026-05-12T00:00:00Z"),
            ]
        );
    }

    // ----- over_time -----

    #[test]
    fn over_time_empty_window_returns_empty_buckets() {
        let resp = compute_patches_over_time(
            &[],
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-10T00:00:00Z"),
            BucketGranularity::Day,
        );
        assert!(resp.buckets.is_empty());
    }

    #[test]
    fn over_time_counts_creation_and_merge_in_correct_buckets() {
        let from = dt("2026-05-10T00:00:00Z");
        let to = dt("2026-05-13T00:00:00Z");
        let repo_a = repo("dourolabs/hydra");

        // Patch 1: created on day 0, merged on day 2.
        let p1 = vec![
            versioned(
                patch_with_status(PatchStatus::Open, "alice", repo_a.clone(), false),
                1,
                dt("2026-05-10T08:00:00Z"),
            ),
            versioned(
                patch_with_status(PatchStatus::Merged, "alice", repo_a.clone(), false),
                2,
                dt("2026-05-12T09:00:00Z"),
            ),
        ];

        // Patch 2: created on day 1, never merged.
        let p2 = vec![versioned(
            patch_with_status(PatchStatus::Open, "bob", repo_a.clone(), false),
            1,
            dt("2026-05-11T08:00:00Z"),
        )];

        // Patch 3: created BEFORE window, merged INSIDE window on day 1.
        let p3 = vec![
            versioned(
                patch_with_status(PatchStatus::Open, "carol", repo_a.clone(), false),
                1,
                dt("2026-05-05T08:00:00Z"),
            ),
            versioned(
                patch_with_status(PatchStatus::Merged, "carol", repo_a.clone(), false),
                2,
                dt("2026-05-11T10:00:00Z"),
            ),
        ];

        let histories = vec![history(p1), history(p2), history(p3)];
        let resp = compute_patches_over_time(&histories, from, to, BucketGranularity::Day);

        assert_eq!(resp.buckets.len(), 3);
        assert_eq!(resp.buckets[0].bucket_start, dt("2026-05-10T00:00:00Z"));
        assert_eq!(resp.buckets[0].created, 1);
        assert_eq!(resp.buckets[0].merged, 0);
        assert_eq!(resp.buckets[1].bucket_start, dt("2026-05-11T00:00:00Z"));
        assert_eq!(resp.buckets[1].created, 1);
        assert_eq!(resp.buckets[1].merged, 1);
        assert_eq!(resp.buckets[2].bucket_start, dt("2026-05-12T00:00:00Z"));
        assert_eq!(resp.buckets[2].created, 0);
        assert_eq!(resp.buckets[2].merged, 1);
    }

    #[test]
    fn over_time_excludes_events_outside_window() {
        let from = dt("2026-05-10T00:00:00Z");
        let to = dt("2026-05-12T00:00:00Z");
        let repo_a = repo("dourolabs/hydra");
        // Created before window, merged after window.
        let p = vec![
            versioned(
                patch_with_status(PatchStatus::Open, "alice", repo_a.clone(), false),
                1,
                dt("2026-05-01T08:00:00Z"),
            ),
            versioned(
                patch_with_status(PatchStatus::Merged, "alice", repo_a, false),
                2,
                dt("2026-06-01T08:00:00Z"),
            ),
        ];
        let histories = vec![history(p)];
        let resp = compute_patches_over_time(&histories, from, to, BucketGranularity::Day);
        assert_eq!(resp.buckets.len(), 2);
        for bucket in &resp.buckets {
            assert_eq!(bucket.created, 0);
            assert_eq!(bucket.merged, 0);
        }
    }

    // ----- terminal_mix -----

    #[test]
    fn terminal_mix_counts_first_terminal_flip_per_patch() {
        let from = dt("2026-05-10T00:00:00Z");
        let to = dt("2026-05-13T00:00:00Z");
        let repo_a = repo("dourolabs/hydra");

        // Three patches: one merged in window, one closed in window,
        // one merged after window (excluded).
        let p1 = vec![
            versioned(
                patch_with_status(PatchStatus::Open, "alice", repo_a.clone(), false),
                1,
                dt("2026-05-09T00:00:00Z"),
            ),
            versioned(
                patch_with_status(PatchStatus::Merged, "alice", repo_a.clone(), false),
                2,
                dt("2026-05-10T12:00:00Z"),
            ),
        ];
        let p2 = vec![
            versioned(
                patch_with_status(PatchStatus::Open, "bob", repo_a.clone(), false),
                1,
                dt("2026-05-09T00:00:00Z"),
            ),
            versioned(
                patch_with_status(PatchStatus::Closed, "bob", repo_a.clone(), false),
                2,
                dt("2026-05-11T12:00:00Z"),
            ),
        ];
        let p3 = vec![
            versioned(
                patch_with_status(PatchStatus::Open, "carol", repo_a.clone(), false),
                1,
                dt("2026-05-09T00:00:00Z"),
            ),
            versioned(
                patch_with_status(PatchStatus::Merged, "carol", repo_a, false),
                2,
                dt("2026-06-01T00:00:00Z"),
            ),
        ];

        let histories = vec![history(p1), history(p2), history(p3)];
        let resp = compute_patches_terminal_mix(&histories, from, to);
        assert_eq!(resp.merged, 1);
        assert_eq!(resp.closed, 1);
    }

    #[test]
    fn terminal_mix_ignores_open_and_changes_requested_patches() {
        let from = dt("2026-05-10T00:00:00Z");
        let to = dt("2026-05-13T00:00:00Z");
        let repo_a = repo("dourolabs/hydra");
        let p = vec![versioned(
            patch_with_status(PatchStatus::Open, "alice", repo_a, false),
            1,
            dt("2026-05-11T00:00:00Z"),
        )];
        let resp = compute_patches_terminal_mix(&[history(p)], from, to);
        assert_eq!(resp.merged, 0);
        assert_eq!(resp.closed, 0);
    }

    // ----- time_to_merge -----

    #[test]
    fn time_to_merge_zero_samples_yields_none_percentiles() {
        let from = dt("2026-05-10T00:00:00Z");
        let to = dt("2026-05-13T00:00:00Z");
        let resp = compute_patches_time_to_merge(&[], from, to);
        assert_eq!(resp.count, 0);
        assert!(resp.median_seconds.is_none());
        assert!(resp.p95_seconds.is_none());
        // Histogram is always emitted with 0-count bins.
        assert!(!resp.histogram.is_empty());
        for bin in &resp.histogram {
            assert_eq!(bin.count, 0);
        }
    }

    #[test]
    fn time_to_merge_single_sample_collapses_median_and_p95() {
        let from = dt("2026-05-10T00:00:00Z");
        let to = dt("2026-05-13T00:00:00Z");
        let repo_a = repo("dourolabs/hydra");
        let p = vec![
            versioned(
                patch_with_status(PatchStatus::Open, "alice", repo_a.clone(), false),
                1,
                dt("2026-05-10T00:00:00Z"),
            ),
            versioned(
                patch_with_status(PatchStatus::Merged, "alice", repo_a, false),
                2,
                dt("2026-05-10T03:00:00Z"),
            ),
        ];
        let resp = compute_patches_time_to_merge(&[history(p)], from, to);
        assert_eq!(resp.count, 1);
        assert_eq!(resp.median_seconds, Some(10_800));
        assert_eq!(resp.p95_seconds, Some(10_800));
        // 3h falls in [1h, 4h).
        let bin = resp
            .histogram
            .iter()
            .find(|b| b.bin_start_seconds == 3_600)
            .expect("1h bin");
        assert_eq!(bin.count, 1);
    }

    #[test]
    fn time_to_merge_two_equal_samples_share_percentiles() {
        let from = dt("2026-05-10T00:00:00Z");
        let to = dt("2026-05-13T00:00:00Z");
        let repo_a = repo("dourolabs/hydra");
        let mk = |t0: &str, t1: &str| {
            vec![
                versioned(
                    patch_with_status(PatchStatus::Open, "alice", repo_a.clone(), false),
                    1,
                    dt(t0),
                ),
                versioned(
                    patch_with_status(PatchStatus::Merged, "alice", repo_a.clone(), false),
                    2,
                    dt(t1),
                ),
            ]
        };
        let p1 = mk("2026-05-10T00:00:00Z", "2026-05-10T01:00:00Z");
        let p2 = mk("2026-05-10T00:00:00Z", "2026-05-10T01:00:00Z");
        let resp = compute_patches_time_to_merge(&[history(p1), history(p2)], from, to);
        assert_eq!(resp.count, 2);
        assert_eq!(resp.median_seconds, Some(3_600));
        assert_eq!(resp.p95_seconds, Some(3_600));
    }

    #[test]
    fn time_to_merge_excludes_merges_outside_window() {
        let from = dt("2026-05-10T00:00:00Z");
        let to = dt("2026-05-13T00:00:00Z");
        let repo_a = repo("dourolabs/hydra");
        let p = vec![
            versioned(
                patch_with_status(PatchStatus::Open, "alice", repo_a.clone(), false),
                1,
                dt("2026-05-01T00:00:00Z"),
            ),
            versioned(
                patch_with_status(PatchStatus::Merged, "alice", repo_a, false),
                2,
                dt("2026-06-01T00:00:00Z"),
            ),
        ];
        let resp = compute_patches_time_to_merge(&[history(p)], from, to);
        assert_eq!(resp.count, 0);
    }

    // ----- in_flight_over_time -----

    #[test]
    fn in_flight_counts_patches_open_at_each_boundary() {
        let from = dt("2026-05-10T00:00:00Z");
        let to = dt("2026-05-13T00:00:00Z");
        let repo_a = repo("dourolabs/hydra");

        // p1: open 2026-05-09 → merged 2026-05-11. In-flight on the
        // 10th, not on the 11th or 12th.
        let p1 = vec![
            versioned(
                patch_with_status(PatchStatus::Open, "alice", repo_a.clone(), false),
                1,
                dt("2026-05-09T00:00:00Z"),
            ),
            versioned(
                patch_with_status(PatchStatus::Merged, "alice", repo_a.clone(), false),
                2,
                dt("2026-05-11T05:00:00Z"),
            ),
        ];

        // p2: open on 2026-05-12, still open at end. In-flight only
        // on day 2 (2026-05-12) since creation precedes that boundary.
        let p2 = vec![versioned(
            patch_with_status(PatchStatus::Open, "bob", repo_a.clone(), false),
            1,
            dt("2026-05-12T08:00:00Z"),
        )];
        // NOTE: p2 created at 08:00 on 2026-05-12 — at midnight of
        // 2026-05-12 the patch does NOT yet exist, so it doesn't
        // count for that bucket.

        // p3: changes_requested on 2026-05-10 → still changes_requested.
        let p3 = vec![versioned(
            patch_with_status(PatchStatus::ChangesRequested, "carol", repo_a, false),
            1,
            dt("2026-05-09T00:00:00Z"),
        )];

        let histories = vec![history(p1), history(p2), history(p3)];
        let resp =
            compute_patches_in_flight_over_time(&histories, from, to, BucketGranularity::Day);
        assert_eq!(resp.buckets.len(), 3);

        // Day 0 (2026-05-10): p1 open + p3 changes-requested = 2.
        assert_eq!(resp.buckets[0].in_flight, 2);
        // Day 1 (2026-05-11): only p3 (p1 was merged at 05:00 > midnight; so at midnight p1 still open).
        // Wait — bucket_start is at midnight 2026-05-11. p1 merged at 05:00. So at midnight p1 was still open.
        assert_eq!(resp.buckets[1].in_flight, 2);
        // Day 2 (2026-05-12): p1 now merged (yesterday). p2 not yet
        // (08:00 > midnight). p3 still in flight. = 1.
        assert_eq!(resp.buckets[2].in_flight, 1);
    }

    /// End-to-end fixture: the five-patch scenario the issue spec
    /// requires for acceptance.
    ///
    /// - p_in_window: created + merged inside the window.
    /// - p_open_in_window: created inside the window, not yet merged.
    /// - p_pre_window_merged: created before window, merged inside.
    /// - p_closed_in_window: closed-without-merging inside window.
    /// - p_deleted: deleted patch — excluded by fetch_patch_histories
    ///   (not exercised here since we test the aggregator's pure
    ///   inputs; the fetcher's exclusion is covered separately).
    #[test]
    fn over_time_terminal_mix_and_time_to_merge_align_on_spec_fixture() {
        let from = dt("2026-05-10T00:00:00Z");
        let to = dt("2026-05-17T00:00:00Z");
        let repo_a = repo("dourolabs/hydra");

        let p_in_window = vec![
            versioned(
                patch_with_status(PatchStatus::Open, "alice", repo_a.clone(), false),
                1,
                dt("2026-05-11T00:00:00Z"),
            ),
            versioned(
                patch_with_status(PatchStatus::Merged, "alice", repo_a.clone(), false),
                2,
                dt("2026-05-12T00:00:00Z"),
            ),
        ];
        let p_open_in_window = vec![versioned(
            patch_with_status(PatchStatus::Open, "bob", repo_a.clone(), false),
            1,
            dt("2026-05-13T00:00:00Z"),
        )];
        let p_pre_window_merged = vec![
            versioned(
                patch_with_status(PatchStatus::Open, "carol", repo_a.clone(), false),
                1,
                dt("2026-05-01T00:00:00Z"),
            ),
            versioned(
                patch_with_status(PatchStatus::Merged, "carol", repo_a.clone(), false),
                2,
                dt("2026-05-14T00:00:00Z"),
            ),
        ];
        let p_closed_in_window = vec![
            versioned(
                patch_with_status(PatchStatus::Open, "dave", repo_a.clone(), false),
                1,
                dt("2026-05-11T00:00:00Z"),
            ),
            versioned(
                patch_with_status(PatchStatus::Closed, "dave", repo_a, false),
                2,
                dt("2026-05-15T00:00:00Z"),
            ),
        ];
        let histories = vec![
            history(p_in_window),
            history(p_open_in_window),
            history(p_pre_window_merged),
            history(p_closed_in_window),
        ];

        // over_time: 3 created in window, 2 merged in window.
        let over = compute_patches_over_time(&histories, from, to, BucketGranularity::Day);
        let total_created: u64 = over.buckets.iter().map(|b| b.created).sum();
        let total_merged: u64 = over.buckets.iter().map(|b| b.merged).sum();
        assert_eq!(total_created, 3); // p_in_window, p_open_in_window, p_closed_in_window
        assert_eq!(total_merged, 2); // p_in_window, p_pre_window_merged

        // terminal_mix: merged=2, closed=1.
        let mix = compute_patches_terminal_mix(&histories, from, to);
        assert_eq!(mix.merged, 2);
        assert_eq!(mix.closed, 1);

        // time_to_merge: 2 samples (p_in_window 24h, p_pre_window_merged 13d).
        let ttm = compute_patches_time_to_merge(&histories, from, to);
        assert_eq!(ttm.count, 2);
        assert!(ttm.median_seconds.is_some());

        // in_flight on 2026-05-13 midnight: p_in_window (now merged) -> no.
        //   p_open_in_window (created 2026-05-13T00:00:00Z, at midnight
        //   the version timestamp <= midnight so it IS in flight).
        //   p_pre_window_merged still Open (merge is on 05-14) -> yes.
        //   p_closed_in_window still Open (close on 05-15) -> yes.
        // = 3
        let inflight =
            compute_patches_in_flight_over_time(&histories, from, to, BucketGranularity::Day);
        let day_05_13 = inflight
            .buckets
            .iter()
            .find(|b| b.bucket_start == dt("2026-05-13T00:00:00Z"))
            .expect("2026-05-13 bucket");
        assert_eq!(day_05_13.in_flight, 3);
    }

    #[test]
    fn bin_index_for_falls_into_open_ended_last_bin_above_threshold() {
        // 31 days > 30d edge → last bin.
        let very_long = 31 * 24 * 3_600;
        assert_eq!(bin_index_for(very_long), DURATION_BIN_EDGES.len() - 1);
        // 30 minutes → first bin.
        assert_eq!(bin_index_for(30 * 60), 0);
    }

    #[tokio::test]
    async fn fetch_patch_histories_excludes_automatic_backup_and_deleted() {
        use crate::test_utils::test_state_handles;

        let handles = test_state_handles();
        let store = handles.store.clone();
        let actor = CommonActorRef::test();
        let repo_a = repo("dourolabs/hydra");

        // Regular patch — included.
        let normal = patch_with_status(PatchStatus::Open, "alice", repo_a.clone(), false);
        let (normal_id, _) = store.add_patch(normal, &actor).await.expect("add normal");

        // is_automatic_backup patch — excluded.
        let backup = patch_with_status(PatchStatus::Open, "alice", repo_a.clone(), true);
        let (_backup_id, _) = store.add_patch(backup, &actor).await.expect("add backup");

        // Deleted patch — excluded.
        let to_delete = patch_with_status(PatchStatus::Open, "alice", repo_a, false);
        let (delete_id, _) = store
            .add_patch(to_delete, &actor)
            .await
            .expect("add to_delete");
        store
            .delete_patch(&delete_id, &actor)
            .await
            .expect("delete");

        let query =
            PatchesThroughputQuery::new(dt("2026-05-10T00:00:00Z"), dt("2026-05-13T00:00:00Z"));
        let histories = fetch_patch_histories(store.as_ref(), &query)
            .await
            .expect("fetch");
        let ids: Vec<_> = histories.iter().map(|h| h.patch_id.clone()).collect();
        assert_eq!(ids, vec![normal_id]);
    }

    #[tokio::test]
    async fn fetch_patch_histories_respects_repo_and_creator_filters() {
        use crate::test_utils::test_state_handles;

        let handles = test_state_handles();
        let store = handles.store.clone();
        let actor = CommonActorRef::test();
        let repo_a = repo("dourolabs/hydra");
        let repo_b = repo("other/repo");

        let alice_a = patch_with_status(PatchStatus::Open, "alice", repo_a.clone(), false);
        let (alice_a_id, _) = store.add_patch(alice_a, &actor).await.expect("alice_a");
        let bob_a = patch_with_status(PatchStatus::Open, "bob", repo_a, false);
        let (_bob_a_id, _) = store.add_patch(bob_a, &actor).await.expect("bob_a");
        let alice_b = patch_with_status(PatchStatus::Open, "alice", repo_b, false);
        let (_alice_b_id, _) = store.add_patch(alice_b, &actor).await.expect("alice_b");

        let mut query =
            PatchesThroughputQuery::new(dt("2026-05-10T00:00:00Z"), dt("2026-05-13T00:00:00Z"));
        query.repo_name = Some("dourolabs/hydra".to_string());
        query.creator = Some("alice".to_string());
        let histories = fetch_patch_histories(store.as_ref(), &query)
            .await
            .expect("fetch");
        let ids: Vec<_> = histories.iter().map(|h| h.patch_id.clone()).collect();
        assert_eq!(ids, vec![alice_a_id]);
    }

    // ----- issue analytics -----

    use crate::domain::issues::{
        Issue as DomainIssue, IssueType as DomainIssueType, SessionSettings,
    };
    use crate::domain::projects::{default_project_id, default_project_seed};
    use hydra_common::api::v1::projects::StatusKey as ApiStatusKey;

    fn skey(s: &str) -> ApiStatusKey {
        ApiStatusKey::try_new(s).expect("status key")
    }

    fn issue_in_default_project(status: &str, creator: &str) -> DomainIssue {
        issue_in_default_project_typed(status, creator, DomainIssueType::Task)
    }

    fn issue_in_default_project_typed(
        status: &str,
        creator: &str,
        issue_type: DomainIssueType,
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

    #[tokio::test]
    async fn fetch_issue_histories_filters_by_issue_types() {
        use crate::test_utils::test_state_handles;
        use hydra_common::api::v1::issues::IssueType as ApiIssueType;
        let handles = test_state_handles();
        let store = handles.store.clone();
        let actor = CommonActorRef::test();

        let feature = issue_in_default_project_typed("open", "alice", DomainIssueType::Feature);
        let (feature_id, _) = store.add_issue(feature, &actor).await.expect("feature");
        let bug = issue_in_default_project_typed("open", "alice", DomainIssueType::Bug);
        let (bug_id, _) = store.add_issue(bug, &actor).await.expect("bug");
        let chore = issue_in_default_project_typed("open", "alice", DomainIssueType::Chore);
        let (_, _) = store.add_issue(chore, &actor).await.expect("chore");

        let window =
            || IssuesThroughputQuery::new(dt("2026-05-10T00:00:00Z"), dt("2026-05-13T00:00:00Z"));

        // (a) plural set of 2 types narrows the cohort to those types only.
        let mut q = window();
        q.issue_types = vec![ApiIssueType::Feature, ApiIssueType::Bug];
        let mut ids: Vec<_> = fetch_issue_histories(store.as_ref(), &q)
            .await
            .expect("fetch")
            .into_iter()
            .map(|h| h.issue_id)
            .collect();
        ids.sort();
        let mut expected = vec![feature_id.clone(), bug_id.clone()];
        expected.sort();
        assert_eq!(ids, expected);

        // (b) empty plural + singular set still filters by the singular value.
        let mut q = window();
        q.issue_type = Some(ApiIssueType::Feature);
        let ids: Vec<_> = fetch_issue_histories(store.as_ref(), &q)
            .await
            .expect("fetch")
            .into_iter()
            .map(|h| h.issue_id)
            .collect();
        assert_eq!(ids, vec![feature_id.clone()]);

        // (c) plural with the same type as singular yields the same cohort as
        //     plural-only (plural wins; singular is ignored when plural is set).
        let mut q = window();
        q.issue_type = Some(ApiIssueType::Bug);
        q.issue_types = vec![ApiIssueType::Feature];
        let ids: Vec<_> = fetch_issue_histories(store.as_ref(), &q)
            .await
            .expect("fetch")
            .into_iter()
            .map(|h| h.issue_id)
            .collect();
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
