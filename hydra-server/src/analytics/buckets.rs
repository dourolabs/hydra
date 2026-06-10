use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use hydra_common::api::v1::analytics::{BucketGranularity, TimeToMergeBin};

/// Snap a timestamp down to the start of its enclosing bucket.
///
/// - `Day` buckets are UTC midnight of the same date.
/// - `Week` buckets are Monday UTC 00:00 (ISO weeks).
pub(super) fn floor_to_bucket(t: DateTime<Utc>, bucket: BucketGranularity) -> DateTime<Utc> {
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

pub(super) fn to_utc_midnight(date: NaiveDate) -> DateTime<Utc> {
    let naive = NaiveDateTime::new(
        date,
        NaiveTime::from_hms_opt(0, 0, 0).expect("midnight is a valid time"),
    );
    DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc)
}

pub(super) fn step(bucket: BucketGranularity) -> Duration {
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
pub(super) fn bucket_starts(
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

/// Fixed histogram bin edges (in seconds) shared by the patches
/// `time_to_merge` and issues `cycle_time` endpoints. The final bin has
/// no upper bound; everything `>= last edge` lands in it. Documented on
/// each response type.
pub(super) const DURATION_BIN_EDGES: &[u64] = &[
    0, 3_600,     // 1h
    14_400,    // 4h
    86_400,    // 1d
    259_200,   // 3d
    604_800,   // 7d
    1_209_600, // 14d
    2_592_000, // 30d
];

pub(super) fn empty_duration_histogram() -> Vec<TimeToMergeBin> {
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

pub(super) fn bin_index_for(seconds: u64) -> usize {
    // The final open-ended bin owns anything >= last edge.
    for (i, window) in DURATION_BIN_EDGES.windows(2).enumerate() {
        if seconds < window[1] {
            return i;
        }
    }
    DURATION_BIN_EDGES.len() - 1
}

pub(super) fn percentile(sorted: &[u64], p: f64) -> Option<u64> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .expect("rfc3339 timestamp")
            .with_timezone(&Utc)
    }

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

    #[test]
    fn bin_index_for_falls_into_open_ended_last_bin_above_threshold() {
        // 31 days > 30d edge → last bin.
        let very_long = 31 * 24 * 3_600;
        assert_eq!(bin_index_for(very_long), DURATION_BIN_EDGES.len() - 1);
        // 30 minutes → first bin.
        assert_eq!(bin_index_for(30 * 60), 0);
    }
}
