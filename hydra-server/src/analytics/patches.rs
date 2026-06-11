use super::buckets::{bin_index_for, bucket_starts, empty_duration_histogram, percentile, step};
use crate::domain::patches::{Patch, PatchStatus};
use crate::store::{ReadOnlyStore, StoreError};
use chrono::{DateTime, Utc};
use hydra_common::api::v1::analytics::{
    BucketGranularity, PatchInFlightBucket, PatchOverTimeBucket, PatchesInFlightOverTimeResponse,
    PatchesOverTimeResponse, PatchesTerminalMixResponse, PatchesThroughputQuery,
    PatchesTimeToMergeResponse, TimeToMergeBin,
};
use hydra_common::api::v1::pagination::compute_next_cursor;
use hydra_common::api::v1::patches::SearchPatchesQuery;
use hydra_common::{PatchId, Versioned};

/// Batch size used when streaming patches through the analytics
/// aggregators. Matches [`hydra_common::api::v1::pagination::MAX_LIMIT`]
/// — the same ceiling every other paginated route applies — so callers
/// pay one round-trip per 200 patches instead of materializing the full
/// list.
pub const ANALYTICS_BATCH_SIZE: u32 = 200;

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

/// Stream the patches matching the throughput-query filters through
/// `visit`, one [`PatchHistory`] at a time. The implementation walks
/// [`ReadOnlyStore::list_patches`] in [`ANALYTICS_BATCH_SIZE`]-sized
/// cursor-paged batches so peak memory is bounded by one page of
/// histories — not the full dataset. Deleted patches and
/// `is_automatic_backup` patches are excluded; deleted patches are
/// filtered store-side at the latest version, `is_automatic_backup` is
/// filtered per-row inside the loop because it isn't a list-level
/// filter.
pub async fn for_each_patch_history<F>(
    store: &dyn ReadOnlyStore,
    query: &PatchesThroughputQuery,
    mut visit: F,
) -> Result<(), StoreError>
where
    F: FnMut(&PatchHistory),
{
    let mut search = SearchPatchesQuery::default();
    search.repo_name = query.repo_name.clone();
    search.creator = query.creator.clone();
    search.status = query.status.clone();
    search.limit = Some(ANALYTICS_BATCH_SIZE);

    let mut cursor: Option<String> = None;
    loop {
        search.cursor = cursor.clone();
        let mut page = store.list_patches(&search).await?;
        if page.is_empty() {
            return Ok(());
        }
        // `list_patches` returns up to `limit + 1` rows; `compute_next_cursor`
        // truncates the extra and returns the cursor that resumes the next
        // page, or `None` when the page is the tail.
        let next_cursor = compute_next_cursor(
            &mut page,
            Some(ANALYTICS_BATCH_SIZE),
            |(_id, v)| &v.timestamp,
            |(id, _v)| id.as_ref(),
        );
        for (patch_id, latest) in page {
            if latest.item.is_automatic_backup {
                continue;
            }
            let versions = store.get_patch_versions(&patch_id).await?;
            if versions.is_empty() {
                continue;
            }
            let history = PatchHistory::new(patch_id, versions);
            visit(&history);
        }
        match next_cursor {
            Some(c) => cursor = Some(c),
            None => return Ok(()),
        }
    }
}

/// Streaming accumulator for `patches/over_time`. Per bucket, counts
/// patches whose creation timestamp lands in the bucket and patches
/// whose first transition-to-merged timestamp lands in the bucket.
/// Buckets with zero hits are kept so the frontend gets a dense series.
pub struct PatchesOverTimeAccumulator {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    first_start: Option<DateTime<Utc>>,
    step_secs: i64,
    buckets: Vec<PatchOverTimeBucket>,
}

impl PatchesOverTimeAccumulator {
    pub fn new(from: DateTime<Utc>, to: DateTime<Utc>, bucket: BucketGranularity) -> Self {
        let starts = bucket_starts(from, to, bucket);
        let first_start = starts.first().copied();
        let buckets: Vec<PatchOverTimeBucket> = starts
            .into_iter()
            .map(|s| PatchOverTimeBucket::new(s, 0, 0))
            .collect();
        Self {
            from,
            to,
            first_start,
            step_secs: step(bucket).num_seconds(),
            buckets,
        }
    }

    fn bucket_for(&self, t: DateTime<Utc>) -> Option<usize> {
        if t < self.from || t >= self.to {
            return None;
        }
        let first_start = self.first_start?;
        let delta = (t - first_start).num_seconds();
        let idx = (delta / self.step_secs) as usize;
        if idx >= self.buckets.len() {
            None
        } else {
            Some(idx)
        }
    }

    pub fn fold(&mut self, history: &PatchHistory) {
        if let Some(idx) = self.bucket_for(history.created_at()) {
            self.buckets[idx].created += 1;
        }
        if let Some(merged_at) = history.merged_at() {
            if let Some(idx) = self.bucket_for(merged_at) {
                self.buckets[idx].merged += 1;
            }
        }
    }

    pub fn finalize(self) -> PatchesOverTimeResponse {
        PatchesOverTimeResponse::new(self.buckets)
    }
}

/// Compute `patches/over_time` from an already-materialized slice.
/// Thin wrapper around [`PatchesOverTimeAccumulator`] kept so unit tests
/// can pass hand-rolled fixtures without going through a Store.
pub fn compute_patches_over_time(
    histories: &[PatchHistory],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    bucket: BucketGranularity,
) -> PatchesOverTimeResponse {
    let mut acc = PatchesOverTimeAccumulator::new(from, to, bucket);
    for history in histories {
        acc.fold(history);
    }
    acc.finalize()
}

/// Streaming accumulator for `patches/terminal_mix`. Counts patches by
/// the terminal state they first flipped to, provided that flip falls
/// inside `[from, to)`.
pub struct PatchesTerminalMixAccumulator {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    merged: u64,
    closed: u64,
}

impl PatchesTerminalMixAccumulator {
    pub fn new(from: DateTime<Utc>, to: DateTime<Utc>) -> Self {
        Self {
            from,
            to,
            merged: 0,
            closed: 0,
        }
    }

    pub fn fold(&mut self, history: &PatchHistory) {
        let Some((status, flip_at)) = history.first_terminal() else {
            return;
        };
        if flip_at < self.from || flip_at >= self.to {
            return;
        }
        match status {
            PatchStatus::Merged => self.merged += 1,
            PatchStatus::Closed => self.closed += 1,
            // first_terminal only returns Merged/Closed.
            PatchStatus::Open | PatchStatus::ChangesRequested => {}
        }
    }

    pub fn finalize(self) -> PatchesTerminalMixResponse {
        PatchesTerminalMixResponse::new(self.merged, self.closed)
    }
}

/// Compute `patches/terminal_mix` from an already-materialized slice.
pub fn compute_patches_terminal_mix(
    histories: &[PatchHistory],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> PatchesTerminalMixResponse {
    let mut acc = PatchesTerminalMixAccumulator::new(from, to);
    for history in histories {
        acc.fold(history);
    }
    acc.finalize()
}

/// Streaming accumulator for `patches/time_to_merge`. Builds the
/// histogram and the per-sample delta list for percentile computation.
/// The delta list grows with the in-window merged-patch count, not the
/// total cohort size — accumulator state, not full materialization.
pub struct PatchesTimeToMergeAccumulator {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    deltas: Vec<u64>,
    histogram: Vec<TimeToMergeBin>,
}

impl PatchesTimeToMergeAccumulator {
    pub fn new(from: DateTime<Utc>, to: DateTime<Utc>) -> Self {
        Self {
            from,
            to,
            deltas: Vec::new(),
            histogram: empty_duration_histogram(),
        }
    }

    pub fn fold(&mut self, history: &PatchHistory) {
        let Some(merged_at) = history.merged_at() else {
            return;
        };
        if merged_at < self.from || merged_at >= self.to {
            return;
        }
        let created = history.created_at();
        // A merged_at strictly before created would be a corrupt
        // history; clamp to 0 rather than panic so analytics stays
        // best-effort.
        let delta = (merged_at - created).num_seconds().max(0) as u64;
        self.deltas.push(delta);
        let idx = bin_index_for(delta);
        self.histogram[idx].count += 1;
    }

    pub fn finalize(mut self) -> PatchesTimeToMergeResponse {
        self.deltas.sort_unstable();
        let count = self.deltas.len() as u64;
        let median = percentile(&self.deltas, 0.5);
        let p95 = percentile(&self.deltas, 0.95);
        PatchesTimeToMergeResponse::new(median, p95, count, self.histogram)
    }
}

/// Compute `patches/time_to_merge` from an already-materialized slice.
pub fn compute_patches_time_to_merge(
    histories: &[PatchHistory],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> PatchesTimeToMergeResponse {
    let mut acc = PatchesTimeToMergeAccumulator::new(from, to);
    for history in histories {
        acc.fold(history);
    }
    acc.finalize()
}

/// Streaming accumulator for `patches/in_flight_over_time`. Per bucket
/// boundary, counts patches whose status snapshot is `Open` or
/// `ChangesRequested`. Bucket boundaries are pure functions of
/// `[from, to)` + granularity, so they're constant across batches.
pub struct PatchesInFlightOverTimeAccumulator {
    starts: Vec<DateTime<Utc>>,
    counts: Vec<u64>,
}

impl PatchesInFlightOverTimeAccumulator {
    pub fn new(from: DateTime<Utc>, to: DateTime<Utc>, bucket: BucketGranularity) -> Self {
        let starts = bucket_starts(from, to, bucket);
        let counts = vec![0u64; starts.len()];
        Self { starts, counts }
    }

    pub fn fold(&mut self, history: &PatchHistory) {
        for (i, start) in self.starts.iter().enumerate() {
            if matches!(
                history.status_at(*start),
                Some(PatchStatus::Open) | Some(PatchStatus::ChangesRequested)
            ) {
                self.counts[i] += 1;
            }
        }
    }

    pub fn finalize(self) -> PatchesInFlightOverTimeResponse {
        let buckets = self
            .starts
            .into_iter()
            .zip(self.counts)
            .map(|(start, in_flight)| PatchInFlightBucket::new(start, in_flight))
            .collect();
        PatchesInFlightOverTimeResponse::new(buckets)
    }
}

/// Compute `patches/in_flight_over_time` from an already-materialized
/// slice.
pub fn compute_patches_in_flight_over_time(
    histories: &[PatchHistory],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    bucket: BucketGranularity,
) -> PatchesInFlightOverTimeResponse {
    let mut acc = PatchesInFlightOverTimeAccumulator::new(from, to, bucket);
    for history in histories {
        acc.fold(history);
    }
    acc.finalize()
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

    /// Collect every history surfaced by [`for_each_patch_history`] into
    /// a `Vec` for assertion. Only used in tests — production code uses
    /// the streaming accumulators.
    async fn collect_histories(
        store: &dyn ReadOnlyStore,
        query: &PatchesThroughputQuery,
    ) -> Vec<PatchHistory> {
        let mut out: Vec<PatchHistory> = Vec::new();
        for_each_patch_history(store, query, |h| out.push(h.clone()))
            .await
            .expect("for_each_patch_history");
        out
    }

    #[tokio::test]
    async fn for_each_patch_history_excludes_automatic_backup_and_deleted() {
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
        let histories = collect_histories(store.as_ref(), &query).await;
        let ids: Vec<_> = histories.iter().map(|h| h.patch_id.clone()).collect();
        assert_eq!(ids, vec![normal_id]);
    }

    #[tokio::test]
    async fn for_each_patch_history_respects_repo_and_creator_filters() {
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
        let histories = collect_histories(store.as_ref(), &query).await;
        let ids: Vec<_> = histories.iter().map(|h| h.patch_id.clone()).collect();
        assert_eq!(ids, vec![alice_a_id]);
    }

    /// Seed > [`ANALYTICS_BATCH_SIZE`] patches and confirm the batched
    /// driver returns every one in a single sweep. Crosses ≥ 2 cursor
    /// pages, which is the regression bar for the cursor advance.
    #[tokio::test]
    async fn for_each_patch_history_crosses_batch_boundary() {
        use crate::test_utils::test_state_handles;

        let handles = test_state_handles();
        let store = handles.store.clone();
        let actor = CommonActorRef::test();
        let repo_a = repo("dourolabs/hydra");

        // ANALYTICS_BATCH_SIZE + 5 so the driver has to advance the
        // cursor at least once.
        let total = (ANALYTICS_BATCH_SIZE + 5) as usize;
        let mut expected = std::collections::HashSet::new();
        for _ in 0..total {
            let p = patch_with_status(PatchStatus::Open, "alice", repo_a.clone(), false);
            let (id, _) = store.add_patch(p, &actor).await.expect("add patch");
            expected.insert(id);
        }

        let query =
            PatchesThroughputQuery::new(dt("2026-05-10T00:00:00Z"), dt("2026-05-13T00:00:00Z"));
        let histories = collect_histories(store.as_ref(), &query).await;

        let seen: std::collections::HashSet<_> =
            histories.iter().map(|h| h.patch_id.clone()).collect();
        assert_eq!(seen, expected);
        assert_eq!(histories.len(), total);
    }

    /// Drive each accumulator twice over the same seeded store — once
    /// via the batched driver, once via the all-at-once `compute_*`
    /// helper — and assert wire-identical results. Catches batch-boundary
    /// regressions for every aggregator in one shot.
    #[tokio::test]
    async fn accumulators_match_single_pass_across_batch_boundary() {
        use crate::test_utils::test_state_handles;

        let handles = test_state_handles();
        let store = handles.store.clone();
        let actor = CommonActorRef::test();
        let repo_a = repo("dourolabs/hydra");

        // Mix creates / merges / closes spread across the window so each
        // aggregator gets non-trivial input and the cursor advances ≥ once.
        let total = (ANALYTICS_BATCH_SIZE + 50) as usize;
        // Window must straddle `Utc::now()` so every seeded patch
        // (timestamped at add time) lands inside it; otherwise the
        // cross-batch equality checks degenerate to empty == empty.
        let from = dt("2020-01-01T00:00:00Z");
        let to = dt("2100-01-01T00:00:00Z");
        for i in 0..total {
            let mut p = patch_with_status(PatchStatus::Open, "alice", repo_a.clone(), false);
            let (id, _) = store.add_patch(p.clone(), &actor).await.expect("add patch");
            // Every third patch gets merged; every fifth gets closed.
            // The remainder stay open.
            if i % 3 == 0 {
                p.status = PatchStatus::Merged;
                store
                    .update_patch(&id, p.clone(), &actor)
                    .await
                    .expect("merge");
            } else if i % 5 == 0 {
                p.status = PatchStatus::Closed;
                store
                    .update_patch(&id, p.clone(), &actor)
                    .await
                    .expect("close");
            }
        }

        let query = PatchesThroughputQuery::new(from, to);
        let histories = collect_histories(store.as_ref(), &query).await;
        assert!(histories.len() > ANALYTICS_BATCH_SIZE as usize);

        // over_time
        let mut acc = PatchesOverTimeAccumulator::new(from, to, BucketGranularity::Day);
        for_each_patch_history(store.as_ref(), &query, |h| acc.fold(h))
            .await
            .expect("drive over_time");
        assert_eq!(
            acc.finalize(),
            compute_patches_over_time(&histories, from, to, BucketGranularity::Day)
        );

        // terminal_mix
        let mut acc = PatchesTerminalMixAccumulator::new(from, to);
        for_each_patch_history(store.as_ref(), &query, |h| acc.fold(h))
            .await
            .expect("drive terminal_mix");
        assert_eq!(
            acc.finalize(),
            compute_patches_terminal_mix(&histories, from, to)
        );

        // time_to_merge
        let mut acc = PatchesTimeToMergeAccumulator::new(from, to);
        for_each_patch_history(store.as_ref(), &query, |h| acc.fold(h))
            .await
            .expect("drive time_to_merge");
        assert_eq!(
            acc.finalize(),
            compute_patches_time_to_merge(&histories, from, to)
        );

        // in_flight_over_time
        let mut acc = PatchesInFlightOverTimeAccumulator::new(from, to, BucketGranularity::Day);
        for_each_patch_history(store.as_ref(), &query, |h| acc.fold(h))
            .await
            .expect("drive in_flight");
        assert_eq!(
            acc.finalize(),
            compute_patches_in_flight_over_time(&histories, from, to, BucketGranularity::Day)
        );
    }
}
