//! Wire types for the `/v1/analytics` endpoints.
//!
//! See [`super::patches`] / [`super::issues`] for the underlying entity
//! shapes and `hydra-server/src/domain/analytics.rs` for the in-process
//! aggregation that drives these responses.
//!
//! ## "Terminal" — issues
//!
//! The issues endpoints (`/v1/analytics/throughput/issues/...`) treat a
//! status as **terminal** iff its `unblocks_parents` flag is `true` —
//! mirroring the existing helper at
//! `hydra-server/src/policy/restrictions/issue_lifecycle.rs:153`. This
//! groups `closed` with `dropped` / `failed` for the purposes of
//! cycle-time and over-time charts; clients that want to exclude the
//! cancellation lanes can pass `status_keys=closed`.

use super::issues::IssueType;
use super::patches::PatchStatus;
use super::projects::StatusKey;
use super::serde_helpers::{deserialize_comma_separated, serialize_comma_separated};
use crate::ProjectId;
use crate::Rgb;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Time-bucket granularity for analytics time series.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum BucketGranularity {
    #[default]
    Day,
    Week,
}

/// Common query parameters for `/v1/analytics/throughput/patches/...`.
///
/// `from`/`to` are required; the rest are optional filters. `bucket`
/// applies only to time-series endpoints (`over_time`,
/// `in_flight_over_time`); the non-time-series endpoints
/// (`terminal_mix`, `time_to_merge`) accept the field but ignore it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct PatchesThroughputQuery {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    #[serde(default)]
    pub bucket: Option<BucketGranularity>,
    #[serde(default)]
    pub repo_name: Option<String>,
    #[serde(default)]
    pub creator: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_comma_separated",
        deserialize_with = "deserialize_comma_separated"
    )]
    pub status: Vec<PatchStatus>,
}

impl PatchesThroughputQuery {
    pub fn new(from: DateTime<Utc>, to: DateTime<Utc>) -> Self {
        Self {
            from,
            to,
            bucket: None,
            repo_name: None,
            creator: None,
            status: Vec::new(),
        }
    }
}

/// A single bucket on the `over_time` series.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct PatchOverTimeBucket {
    pub bucket_start: DateTime<Utc>,
    pub created: u64,
    pub merged: u64,
}

impl PatchOverTimeBucket {
    pub fn new(bucket_start: DateTime<Utc>, created: u64, merged: u64) -> Self {
        Self {
            bucket_start,
            created,
            merged,
        }
    }
}

/// Response for `GET /v1/analytics/throughput/patches/over_time`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct PatchesOverTimeResponse {
    pub buckets: Vec<PatchOverTimeBucket>,
}

impl PatchesOverTimeResponse {
    pub fn new(buckets: Vec<PatchOverTimeBucket>) -> Self {
        Self { buckets }
    }
}

/// Response for `GET /v1/analytics/throughput/patches/terminal_mix`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct PatchesTerminalMixResponse {
    pub merged: u64,
    pub closed: u64,
}

impl PatchesTerminalMixResponse {
    pub fn new(merged: u64, closed: u64) -> Self {
        Self { merged, closed }
    }
}

/// A single bin in the `time_to_merge` histogram. Half-open `[start, end)`
/// in seconds. The last bin's `bin_end_seconds` is `None` to denote
/// "30d+".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct TimeToMergeBin {
    pub bin_start_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin_end_seconds: Option<u64>,
    pub count: u64,
}

impl TimeToMergeBin {
    pub fn new(bin_start_seconds: u64, bin_end_seconds: Option<u64>, count: u64) -> Self {
        Self {
            bin_start_seconds,
            bin_end_seconds,
            count,
        }
    }
}

/// Response for `GET /v1/analytics/throughput/patches/time_to_merge`.
///
/// `median_seconds` and `p95_seconds` are `None` when `count` is zero.
/// The fixed histogram bin scheme is: `[0,1h)`, `[1h,4h)`, `[4h,1d)`,
/// `[1d,3d)`, `[3d,7d)`, `[7d,14d)`, `[14d,30d)`, `[30d, +inf)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct PatchesTimeToMergeResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub median_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95_seconds: Option<u64>,
    pub count: u64,
    pub histogram: Vec<TimeToMergeBin>,
}

impl PatchesTimeToMergeResponse {
    pub fn new(
        median_seconds: Option<u64>,
        p95_seconds: Option<u64>,
        count: u64,
        histogram: Vec<TimeToMergeBin>,
    ) -> Self {
        Self {
            median_seconds,
            p95_seconds,
            count,
            histogram,
        }
    }
}

/// A single bucket on the `in_flight_over_time` series.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct PatchInFlightBucket {
    pub bucket_start: DateTime<Utc>,
    pub in_flight: u64,
}

impl PatchInFlightBucket {
    pub fn new(bucket_start: DateTime<Utc>, in_flight: u64) -> Self {
        Self {
            bucket_start,
            in_flight,
        }
    }
}

/// Response for `GET /v1/analytics/throughput/patches/in_flight_over_time`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct PatchesInFlightOverTimeResponse {
    pub buckets: Vec<PatchInFlightBucket>,
}

impl PatchesInFlightOverTimeResponse {
    pub fn new(buckets: Vec<PatchInFlightBucket>) -> Self {
        Self { buckets }
    }
}

/// Common query parameters for `/v1/analytics/throughput/issues/...`.
///
/// `from`/`to` are required. `project_id` is optional on `over_time` /
/// `cycle_time` (which aggregate across all projects when omitted), but
/// required on `time_in_status_breakdown` / `per_status_distribution`
/// where the status set is project-scoped and cross-project averages are
/// meaningless. `status_keys` is an *include* set — when populated,
/// only issues whose terminal status appears in the set count toward
/// the cohort.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct IssuesThroughputQuery {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    #[serde(default)]
    pub bucket: Option<BucketGranularity>,
    #[serde(default)]
    pub project_id: Option<ProjectId>,
    #[serde(default)]
    pub repo_name: Option<String>,
    /// Single-select form, retained for backward compat. When
    /// [`Self::issue_types`] is non-empty, this field is ignored.
    #[serde(default)]
    pub issue_type: Option<IssueType>,
    /// Multi-select include-set. When non-empty, an issue passes the
    /// type filter iff its `issue_type` is in this set. When empty,
    /// falls back to the singular [`Self::issue_type`] filter.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_comma_separated",
        deserialize_with = "deserialize_comma_separated"
    )]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub issue_types: Vec<IssueType>,
    /// Principal path form (`users/<name>` / `agents/<name>` /
    /// `external/<sys>/<name>`). Filtered as a string match against the
    /// issue's `assignee` path representation so URL-encoded query
    /// strings survive intact without invoking Principal parsing here.
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub creator: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_comma_separated",
        deserialize_with = "deserialize_comma_separated"
    )]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub status_keys: Vec<StatusKey>,
}

impl IssuesThroughputQuery {
    pub fn new(from: DateTime<Utc>, to: DateTime<Utc>) -> Self {
        Self {
            from,
            to,
            bucket: None,
            project_id: None,
            repo_name: None,
            issue_type: None,
            issue_types: Vec::new(),
            assignee: None,
            creator: None,
            status_keys: Vec::new(),
        }
    }
}

/// Response for `GET /v1/analytics/throughput/issues/cycle_time`.
///
/// Reuses the same bin scheme as
/// [`PatchesTimeToMergeResponse`]: `[0,1h)`, `[1h,4h)`, `[4h,1d)`,
/// `[1d,3d)`, `[3d,7d)`, `[7d,14d)`, `[14d,30d)`, `[30d, +inf)`.
/// `median_seconds` and `p95_seconds` are `None` when `count` is zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct IssuesCycleTimeResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub median_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95_seconds: Option<u64>,
    pub count: u64,
    pub histogram: Vec<TimeToMergeBin>,
}

impl IssuesCycleTimeResponse {
    pub fn new(
        median_seconds: Option<u64>,
        p95_seconds: Option<u64>,
        count: u64,
        histogram: Vec<TimeToMergeBin>,
    ) -> Self {
        Self {
            median_seconds,
            p95_seconds,
            count,
            histogram,
        }
    }
}

/// One status's mean time-in-status across the cohort, with the display
/// props resolved from the project's current status definitions so the
/// frontend can render the segment natively. Returned by
/// `GET /v1/analytics/throughput/issues/time_in_status_breakdown`.
///
/// Status keys are resolved at response time from `metis.statuses`. If
/// a status was renamed between when an issue was in it and now, the
/// label reflects the current name (the `(project_id, sequence)` storage
/// FK keeps the join stable).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct TimeInStatusSegment {
    pub status_key: StatusKey,
    pub label: String,
    pub color: Rgb,
    pub mean_seconds: u64,
}

impl TimeInStatusSegment {
    pub fn new(status_key: StatusKey, label: String, color: Rgb, mean_seconds: u64) -> Self {
        Self {
            status_key,
            label,
            color,
            mean_seconds,
        }
    }
}

/// Response for `GET /v1/analytics/throughput/issues/time_in_status_breakdown`.
///
/// `status_segments` follow the project's ordered status list (the
/// `priority` ordering from `metis.statuses` — the order the project
/// itself uses to render statuses). `issue_count` is the size of the
/// cohort (issues that reached a terminal status within the window).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct IssuesTimeInStatusBreakdownResponse {
    pub project_id: ProjectId,
    pub status_segments: Vec<TimeInStatusSegment>,
    pub issue_count: u64,
}

impl IssuesTimeInStatusBreakdownResponse {
    pub fn new(
        project_id: ProjectId,
        status_segments: Vec<TimeInStatusSegment>,
        issue_count: u64,
    ) -> Self {
        Self {
            project_id,
            status_segments,
            issue_count,
        }
    }
}

/// Percentile callouts for a single status's dwell-time samples.
/// Returned by
/// `GET /v1/analytics/throughput/issues/per_status_distribution`.
///
/// `median_seconds` / `p95_seconds` are `None` when `sample_count` is
/// zero. The cohort is every `(issue, status)` dwell-segment that
/// *ended* inside `[from, to)` — an issue still sitting in a status when
/// the window closes does not contribute.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct PerStatusDistribution {
    pub status_key: StatusKey,
    pub label: String,
    pub color: Rgb,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub median_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95_seconds: Option<u64>,
    pub sample_count: u64,
}

impl PerStatusDistribution {
    pub fn new(
        status_key: StatusKey,
        label: String,
        color: Rgb,
        median_seconds: Option<u64>,
        p95_seconds: Option<u64>,
        sample_count: u64,
    ) -> Self {
        Self {
            status_key,
            label,
            color,
            median_seconds,
            p95_seconds,
            sample_count,
        }
    }
}

/// Response for `GET /v1/analytics/throughput/issues/per_status_distribution`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct IssuesPerStatusDistributionResponse {
    pub project_id: ProjectId,
    pub statuses: Vec<PerStatusDistribution>,
}

impl IssuesPerStatusDistributionResponse {
    pub fn new(project_id: ProjectId, statuses: Vec<PerStatusDistribution>) -> Self {
        Self {
            project_id,
            statuses,
        }
    }
}

/// A single bucket on the issues `over_time` series — mirrors
/// [`PatchOverTimeBucket`]'s `(created, merged)` shape with
/// `reached_terminal` replacing `merged`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct IssueOverTimeBucket {
    pub bucket_start: DateTime<Utc>,
    pub created: u64,
    pub reached_terminal: u64,
}

impl IssueOverTimeBucket {
    pub fn new(bucket_start: DateTime<Utc>, created: u64, reached_terminal: u64) -> Self {
        Self {
            bucket_start,
            created,
            reached_terminal,
        }
    }
}

/// Response for `GET /v1/analytics/throughput/issues/over_time`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct IssuesOverTimeResponse {
    pub buckets: Vec<IssueOverTimeBucket>,
}

impl IssuesOverTimeResponse {
    pub fn new(buckets: Vec<IssueOverTimeBucket>) -> Self {
        Self { buckets }
    }
}
