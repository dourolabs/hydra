//! Wire types for the `/v1/analytics` endpoints.
//!
//! See [`super::patches`] for the underlying patch shape and
//! `hydra-server/src/domain/analytics.rs` for the in-process aggregation
//! that drives these responses.

use super::patches::PatchStatus;
use super::serde_helpers::{deserialize_comma_separated, serialize_comma_separated};
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

impl BucketGranularity {
    pub fn as_str(&self) -> &'static str {
        match self {
            BucketGranularity::Day => "day",
            BucketGranularity::Week => "week",
        }
    }
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
