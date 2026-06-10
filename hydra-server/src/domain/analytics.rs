//! In-process analytics aggregation over patch version histories.
//!
//! Backed by the existing `list_patches` / `get_patch_versions` store
//! primitives — no new `Store`-trait methods, no materialized tables.
//! The aggregation walks each patch's full version history in memory.
//! Past production scale this will need a push-down rewrite, but it
//! buys us a complete feature without a parallel store surface to
//! maintain in lockstep.

use crate::domain::patches::{Patch, PatchStatus};
use crate::store::{ReadOnlyStore, StoreError};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use hydra_common::api::v1::analytics::{
    BucketGranularity, PatchInFlightBucket, PatchOverTimeBucket, PatchesInFlightOverTimeResponse,
    PatchesOverTimeResponse, PatchesTerminalMixResponse, PatchesTimeToMergeResponse,
    TimeToMergeBin,
};
use hydra_common::api::v1::patches::{PatchStatus as ApiPatchStatus, SearchPatchesQuery};
use hydra_common::{PatchId, Versioned};

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

    /// Latest stored version. Histories from the store are always
    /// non-empty (add_patch always creates v1), so the unwrap is the
    /// invariant — callers that build histories by hand must keep it.
    fn latest(&self) -> &Versioned<Patch> {
        self.versions
            .last()
            .expect("patch history must contain at least one version")
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

/// Filters applied to the candidate patch set BEFORE history is walked.
///
/// These mirror the `/v1/patches` query semantics so callers get
/// consistent results between the patch list and the analytics charts.
/// `repo_name` and `creator` map to the matching `SearchPatchesQuery`
/// filters; `status` filters by the latest version's status.
#[derive(Debug, Default, Clone)]
pub struct PatchAnalyticsFilters {
    pub repo_name: Option<String>,
    pub creator: Option<String>,
    pub status: Vec<ApiPatchStatus>,
}

/// Fetch the patches matching `filters` and their full version
/// histories. Deleted patches and `is_automatic_backup` patches are
/// excluded — both checks run against the latest stored version.
pub async fn fetch_patch_histories(
    store: &dyn ReadOnlyStore,
    filters: &PatchAnalyticsFilters,
) -> Result<Vec<PatchHistory>, StoreError> {
    let mut query = SearchPatchesQuery::default();
    query.repo_name = filters.repo_name.clone();
    query.creator = filters.creator.clone();
    query.status = filters.status.clone();
    // `list_patches` already filters out `deleted=true` at latest by
    // default. `is_automatic_backup` isn't a list-level filter, so we
    // drop those in the loop below.

    let patches = store.list_patches(&query).await?;
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

/// Fixed histogram bin edges (in seconds). The final bin has no upper
/// bound; everything `>= last edge` lands in it. Documented on the
/// response type.
const TIME_TO_MERGE_BIN_EDGES: &[u64] = &[
    0, 3_600,     // 1h
    14_400,    // 4h
    86_400,    // 1d
    259_200,   // 3d
    604_800,   // 7d
    1_209_600, // 14d
    2_592_000, // 30d
];

fn empty_histogram() -> Vec<TimeToMergeBin> {
    let mut bins = Vec::with_capacity(TIME_TO_MERGE_BIN_EDGES.len());
    for window in TIME_TO_MERGE_BIN_EDGES.windows(2) {
        bins.push(TimeToMergeBin::new(window[0], Some(window[1]), 0));
    }
    let last = *TIME_TO_MERGE_BIN_EDGES
        .last()
        .expect("bin edge list is non-empty");
    bins.push(TimeToMergeBin::new(last, None, 0));
    bins
}

fn bin_index_for(seconds: u64) -> usize {
    // The final open-ended bin owns anything >= last edge.
    for (i, window) in TIME_TO_MERGE_BIN_EDGES.windows(2).enumerate() {
        if seconds < window[1] {
            return i;
        }
    }
    TIME_TO_MERGE_BIN_EDGES.len() - 1
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
    let mut histogram = empty_histogram();
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

/// Drop histories whose latest version's status doesn't match the
/// caller's status filter. Applied at the analytics layer because the
/// store's status filter matches latest-status too, but route handlers
/// call this on hand-built history lists in tests where the store
/// filter wasn't engaged.
pub fn apply_status_filter(
    histories: Vec<PatchHistory>,
    statuses: &[ApiPatchStatus],
) -> Vec<PatchHistory> {
    if statuses.is_empty() {
        return histories;
    }
    histories
        .into_iter()
        .filter(|h| {
            let latest_status: ApiPatchStatus = h.latest().item.status.into();
            statuses.contains(&latest_status)
        })
        .collect()
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

    fn history(id: &str, versions: Vec<Versioned<Patch>>) -> PatchHistory {
        let patch_id = PatchId::new();
        let _ = id;
        PatchHistory::new(patch_id, versions)
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

        let histories = vec![history("p1", p1), history("p2", p2), history("p3", p3)];
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
        let histories = vec![history("p", p)];
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

        let histories = vec![history("p1", p1), history("p2", p2), history("p3", p3)];
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
        let resp = compute_patches_terminal_mix(&[history("p", p)], from, to);
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
        let resp = compute_patches_time_to_merge(&[history("p", p)], from, to);
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
        let resp = compute_patches_time_to_merge(&[history("p1", p1), history("p2", p2)], from, to);
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
        let resp = compute_patches_time_to_merge(&[history("p", p)], from, to);
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

        let histories = vec![history("p1", p1), history("p2", p2), history("p3", p3)];
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

    // ----- filters -----

    #[test]
    fn apply_status_filter_keeps_matching_latest_status_only() {
        let repo_a = repo("dourolabs/hydra");
        let open = history(
            "open",
            vec![versioned(
                patch_with_status(PatchStatus::Open, "alice", repo_a.clone(), false),
                1,
                dt("2026-05-10T00:00:00Z"),
            )],
        );
        let merged = history(
            "merged",
            vec![versioned(
                patch_with_status(PatchStatus::Merged, "alice", repo_a, false),
                1,
                dt("2026-05-10T00:00:00Z"),
            )],
        );
        let kept = apply_status_filter(vec![open.clone(), merged.clone()], &[ApiPatchStatus::Open]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].latest().item.status, PatchStatus::Open);

        // Empty filter is a no-op.
        let all = apply_status_filter(vec![open, merged], &[]);
        assert_eq!(all.len(), 2);
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
            history("p_in_window", p_in_window),
            history("p_open_in_window", p_open_in_window),
            history("p_pre_window_merged", p_pre_window_merged),
            history("p_closed_in_window", p_closed_in_window),
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
        assert_eq!(bin_index_for(very_long), TIME_TO_MERGE_BIN_EDGES.len() - 1);
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

        let histories = fetch_patch_histories(store.as_ref(), &PatchAnalyticsFilters::default())
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

        let filters = PatchAnalyticsFilters {
            repo_name: Some("dourolabs/hydra".to_string()),
            creator: Some("alice".to_string()),
            status: Vec::new(),
        };
        let histories = fetch_patch_histories(store.as_ref(), &filters)
            .await
            .expect("fetch");
        let ids: Vec<_> = histories.iter().map(|h| h.patch_id.clone()).collect();
        assert_eq!(ids, vec![alice_a_id]);
    }
}
