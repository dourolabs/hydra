//! Route handlers for `/v1/analytics/throughput/patches/...`.
//!
//! Each handler validates the time window, fetches matching patches
//! and their histories via the store primitives, then delegates to the
//! pure aggregator in [`crate::domain::analytics`]. The aggregator
//! holds the entire business logic; the handlers are a thin IO shim
//! plus error mapping.

use crate::app::AppState;
use crate::domain::analytics::{
    compute_patches_in_flight_over_time, compute_patches_over_time, compute_patches_terminal_mix,
    compute_patches_time_to_merge, fetch_patch_histories,
};
use crate::store::StoreError;
use anyhow::anyhow;
use axum::{
    Json,
    extract::{Query, State},
};
use hydra_common::api::v1::{
    ApiError,
    analytics::{
        BucketGranularity, PatchesInFlightOverTimeResponse, PatchesOverTimeResponse,
        PatchesTerminalMixResponse, PatchesThroughputQuery, PatchesTimeToMergeResponse,
    },
};
use tracing::{error, info};

/// Validate the time window and resolve the bucket default.
/// `[from, to)` is required to be a strict forward range; `from >= to`
/// yields 400.
fn validate_query(query: &PatchesThroughputQuery) -> Result<BucketGranularity, ApiError> {
    if query.from >= query.to {
        return Err(ApiError::bad_request("'from' must be strictly before 'to'"));
    }
    Ok(query.bucket.unwrap_or_default())
}

fn map_store_error(err: StoreError) -> ApiError {
    error!(error = %err, "analytics store operation failed");
    ApiError::internal(anyhow!("analytics store error: {err}"))
}

/// `GET /v1/analytics/throughput/patches/over_time`
pub async fn patches_over_time(
    State(state): State<AppState>,
    Query(query): Query<PatchesThroughputQuery>,
) -> Result<Json<PatchesOverTimeResponse>, ApiError> {
    info!(
        from = %query.from,
        to = %query.to,
        bucket = ?query.bucket,
        "analytics.patches_over_time invoked"
    );
    let bucket = validate_query(&query)?;
    let histories = fetch_patch_histories(state.store(), &query)
        .await
        .map_err(map_store_error)?;
    let resp = compute_patches_over_time(&histories, query.from, query.to, bucket);
    Ok(Json(resp))
}

/// `GET /v1/analytics/throughput/patches/terminal_mix`
pub async fn patches_terminal_mix(
    State(state): State<AppState>,
    Query(query): Query<PatchesThroughputQuery>,
) -> Result<Json<PatchesTerminalMixResponse>, ApiError> {
    info!(
        from = %query.from,
        to = %query.to,
        "analytics.patches_terminal_mix invoked"
    );
    validate_query(&query)?;
    let histories = fetch_patch_histories(state.store(), &query)
        .await
        .map_err(map_store_error)?;
    let resp = compute_patches_terminal_mix(&histories, query.from, query.to);
    Ok(Json(resp))
}

/// `GET /v1/analytics/throughput/patches/time_to_merge`
pub async fn patches_time_to_merge(
    State(state): State<AppState>,
    Query(query): Query<PatchesThroughputQuery>,
) -> Result<Json<PatchesTimeToMergeResponse>, ApiError> {
    info!(
        from = %query.from,
        to = %query.to,
        "analytics.patches_time_to_merge invoked"
    );
    validate_query(&query)?;
    let histories = fetch_patch_histories(state.store(), &query)
        .await
        .map_err(map_store_error)?;
    let resp = compute_patches_time_to_merge(&histories, query.from, query.to);
    Ok(Json(resp))
}

/// `GET /v1/analytics/throughput/patches/in_flight_over_time`
pub async fn patches_in_flight_over_time(
    State(state): State<AppState>,
    Query(query): Query<PatchesThroughputQuery>,
) -> Result<Json<PatchesInFlightOverTimeResponse>, ApiError> {
    info!(
        from = %query.from,
        to = %query.to,
        bucket = ?query.bucket,
        "analytics.patches_in_flight_over_time invoked"
    );
    let bucket = validate_query(&query)?;
    let histories = fetch_patch_histories(state.store(), &query)
        .await
        .map_err(map_store_error)?;
    let resp = compute_patches_in_flight_over_time(&histories, query.from, query.to, bucket);
    Ok(Json(resp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_state_handles;
    use hydra_common::api::v1::analytics::PatchesThroughputQuery;

    fn dt(s: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(s)
            .expect("rfc3339 timestamp")
            .with_timezone(&chrono::Utc)
    }

    fn smoke_query() -> PatchesThroughputQuery {
        PatchesThroughputQuery::new(dt("2026-05-10T00:00:00Z"), dt("2026-05-13T00:00:00Z"))
    }

    #[tokio::test]
    async fn over_time_with_empty_store_returns_zero_buckets() {
        let handles = test_state_handles();
        let resp = patches_over_time(State(handles.state), Query(smoke_query()))
            .await
            .expect("handler returns 200")
            .0;
        // Window is 2026-05-10 .. 2026-05-13 -> 3 day buckets, all zero.
        assert_eq!(resp.buckets.len(), 3);
        for b in &resp.buckets {
            assert_eq!(b.created, 0);
            assert_eq!(b.merged, 0);
        }
    }

    #[tokio::test]
    async fn inverted_window_returns_400() {
        let handles = test_state_handles();
        let mut q = smoke_query();
        q.to = q.from;
        let err = patches_over_time(State(handles.state), Query(q))
            .await
            .expect_err("inverted window must 400");
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn terminal_mix_with_empty_store_is_zeroes() {
        let handles = test_state_handles();
        let resp = patches_terminal_mix(State(handles.state), Query(smoke_query()))
            .await
            .expect("ok")
            .0;
        assert_eq!(resp.merged, 0);
        assert_eq!(resp.closed, 0);
    }

    #[tokio::test]
    async fn time_to_merge_with_empty_store_is_empty_histogram() {
        let handles = test_state_handles();
        let resp = patches_time_to_merge(State(handles.state), Query(smoke_query()))
            .await
            .expect("ok")
            .0;
        assert_eq!(resp.count, 0);
        assert!(resp.median_seconds.is_none());
        // Bins exist and are all zero.
        assert!(!resp.histogram.is_empty());
        assert!(resp.histogram.iter().all(|b| b.count == 0));
    }

    #[tokio::test]
    async fn in_flight_with_empty_store_is_dense_zero_series() {
        let handles = test_state_handles();
        let resp = patches_in_flight_over_time(State(handles.state), Query(smoke_query()))
            .await
            .expect("ok")
            .0;
        assert_eq!(resp.buckets.len(), 3);
        for b in &resp.buckets {
            assert_eq!(b.in_flight, 0);
        }
    }
}
