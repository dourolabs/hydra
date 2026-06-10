//! Route handlers for `/v1/analytics/throughput/patches/...`.
//!
//! Each handler validates the time window, fetches matching patches
//! and their histories via the store primitives, then delegates to the
//! pure aggregator in [`crate::domain::analytics`]. The aggregator
//! holds the entire business logic; the handlers are a thin IO shim
//! plus error mapping.

use crate::app::AppState;
use crate::app::projects::{ResolveStatusError, project_cached};
use crate::domain::analytics::{
    compute_issues_cycle_time, compute_issues_over_time, compute_issues_per_status_distribution,
    compute_issues_time_in_status_breakdown, compute_patches_in_flight_over_time,
    compute_patches_over_time, compute_patches_terminal_mix, compute_patches_time_to_merge,
    fetch_issue_histories, fetch_patch_histories, resolve_projects_for_histories,
};
use crate::store::StoreError;
use anyhow::anyhow;
use axum::{
    Json,
    extract::{Query, State},
};
use hydra_common::ProjectId;
use hydra_common::api::v1::{
    ApiError,
    analytics::{
        BucketGranularity, IssuesCycleTimeResponse, IssuesOverTimeResponse,
        IssuesPerStatusDistributionResponse, IssuesThroughputQuery,
        IssuesTimeInStatusBreakdownResponse, PatchesInFlightOverTimeResponse,
        PatchesOverTimeResponse, PatchesTerminalMixResponse, PatchesThroughputQuery,
        PatchesTimeToMergeResponse,
    },
    projects::Project,
};
use std::collections::HashMap;
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

fn validate_issues_query(query: &IssuesThroughputQuery) -> Result<BucketGranularity, ApiError> {
    if query.from >= query.to {
        return Err(ApiError::bad_request("'from' must be strictly before 'to'"));
    }
    Ok(query.bucket.unwrap_or_default())
}

/// Fetch the [`Project`] for the supplied id, returning a 400 if it
/// isn't declared (vs. a 500 the way the generic `map_store_error` would
/// surface it).
async fn fetch_project_or_400(
    state: &AppState,
    project_id: &ProjectId,
) -> Result<Project, ApiError> {
    let mut cache: HashMap<ProjectId, Project> = HashMap::new();
    match project_cached(&mut cache, state.store(), project_id).await {
        Ok(_) => Ok(cache.remove(project_id).expect("inserted above")),
        Err(ResolveStatusError::ProjectNotFound(_)) => Err(ApiError::bad_request(format!(
            "project '{project_id}' not found"
        ))),
        Err(ResolveStatusError::Store(err)) => Err(map_store_error(err)),
        Err(other) => Err(ApiError::internal(anyhow!(
            "analytics project resolve error: {other}"
        ))),
    }
}

/// `GET /v1/analytics/throughput/issues/cycle_time`
pub async fn issues_cycle_time(
    State(state): State<AppState>,
    Query(query): Query<IssuesThroughputQuery>,
) -> Result<Json<IssuesCycleTimeResponse>, ApiError> {
    info!(
        from = %query.from,
        to = %query.to,
        project_id = ?query.project_id,
        "analytics.issues_cycle_time invoked"
    );
    validate_issues_query(&query)?;
    let histories = fetch_issue_histories(state.store(), &query)
        .await
        .map_err(map_store_error)?;
    let projects = resolve_projects_for_histories(state.store(), &histories)
        .await
        .map_err(map_store_error)?;
    let resp = compute_issues_cycle_time(
        &histories,
        &projects,
        query.from,
        query.to,
        &query.status_keys,
    );
    Ok(Json(resp))
}

/// `GET /v1/analytics/throughput/issues/over_time`
pub async fn issues_over_time(
    State(state): State<AppState>,
    Query(query): Query<IssuesThroughputQuery>,
) -> Result<Json<IssuesOverTimeResponse>, ApiError> {
    info!(
        from = %query.from,
        to = %query.to,
        bucket = ?query.bucket,
        project_id = ?query.project_id,
        "analytics.issues_over_time invoked"
    );
    let bucket = validate_issues_query(&query)?;
    let histories = fetch_issue_histories(state.store(), &query)
        .await
        .map_err(map_store_error)?;
    let projects = resolve_projects_for_histories(state.store(), &histories)
        .await
        .map_err(map_store_error)?;
    let resp = compute_issues_over_time(
        &histories,
        &projects,
        query.from,
        query.to,
        bucket,
        &query.status_keys,
    );
    Ok(Json(resp))
}

/// `GET /v1/analytics/throughput/issues/time_in_status_breakdown`
pub async fn issues_time_in_status_breakdown(
    State(state): State<AppState>,
    Query(query): Query<IssuesThroughputQuery>,
) -> Result<Json<IssuesTimeInStatusBreakdownResponse>, ApiError> {
    info!(
        from = %query.from,
        to = %query.to,
        project_id = ?query.project_id,
        "analytics.issues_time_in_status_breakdown invoked"
    );
    validate_issues_query(&query)?;
    let project_id = query.project_id.clone().ok_or_else(|| {
        ApiError::bad_request(
            "project_id is required for time_in_status_breakdown (status set is project-scoped)",
        )
    })?;
    let project = fetch_project_or_400(&state, &project_id).await?;
    let histories = fetch_issue_histories(state.store(), &query)
        .await
        .map_err(map_store_error)?;
    let resp = compute_issues_time_in_status_breakdown(
        &histories,
        &project_id,
        &project,
        query.from,
        query.to,
        &query.status_keys,
    );
    Ok(Json(resp))
}

/// `GET /v1/analytics/throughput/issues/per_status_distribution`
pub async fn issues_per_status_distribution(
    State(state): State<AppState>,
    Query(query): Query<IssuesThroughputQuery>,
) -> Result<Json<IssuesPerStatusDistributionResponse>, ApiError> {
    info!(
        from = %query.from,
        to = %query.to,
        project_id = ?query.project_id,
        "analytics.issues_per_status_distribution invoked"
    );
    validate_issues_query(&query)?;
    let project_id = query.project_id.clone().ok_or_else(|| {
        ApiError::bad_request(
            "project_id is required for per_status_distribution (status set is project-scoped)",
        )
    })?;
    let project = fetch_project_or_400(&state, &project_id).await?;
    let histories = fetch_issue_histories(state.store(), &query)
        .await
        .map_err(map_store_error)?;
    let resp = compute_issues_per_status_distribution(
        &histories,
        &project_id,
        &project,
        query.from,
        query.to,
        &query.status_keys,
    );
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

    fn smoke_issues_query() -> IssuesThroughputQuery {
        IssuesThroughputQuery::new(dt("2026-05-10T00:00:00Z"), dt("2026-05-13T00:00:00Z"))
    }

    #[tokio::test]
    async fn issues_cycle_time_with_empty_store_is_zero() {
        let handles = test_state_handles();
        let resp = issues_cycle_time(State(handles.state), Query(smoke_issues_query()))
            .await
            .expect("ok")
            .0;
        assert_eq!(resp.count, 0);
        assert!(resp.median_seconds.is_none());
        assert!(!resp.histogram.is_empty());
    }

    #[tokio::test]
    async fn issues_over_time_with_empty_store_is_dense_zero_series() {
        let handles = test_state_handles();
        let resp = issues_over_time(State(handles.state), Query(smoke_issues_query()))
            .await
            .expect("ok")
            .0;
        assert_eq!(resp.buckets.len(), 3);
        for b in &resp.buckets {
            assert_eq!(b.created, 0);
            assert_eq!(b.reached_terminal, 0);
        }
    }

    #[tokio::test]
    async fn issues_inverted_window_returns_400() {
        let handles = test_state_handles();
        let mut q = smoke_issues_query();
        q.to = q.from;
        let err = issues_cycle_time(State(handles.state), Query(q))
            .await
            .expect_err("inverted window must 400");
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn time_in_status_breakdown_without_project_id_returns_400() {
        let handles = test_state_handles();
        let err =
            issues_time_in_status_breakdown(State(handles.state), Query(smoke_issues_query()))
                .await
                .expect_err("missing project_id must 400");
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn per_status_distribution_without_project_id_returns_400() {
        let handles = test_state_handles();
        let err = issues_per_status_distribution(State(handles.state), Query(smoke_issues_query()))
            .await
            .expect_err("missing project_id must 400");
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn time_in_status_breakdown_with_unknown_project_id_returns_400() {
        let handles = test_state_handles();
        let mut q = smoke_issues_query();
        q.project_id = Some(
            hydra_common::ProjectId::try_from("j-doesnt".to_string())
                .expect("valid project id shape"),
        );
        let err = issues_time_in_status_breakdown(State(handles.state), Query(q))
            .await
            .expect_err("unknown project_id must 400");
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn time_in_status_breakdown_with_default_project_returns_empty_cohort() {
        let handles = test_state_handles();
        let mut q = smoke_issues_query();
        q.project_id = Some(crate::domain::projects::default_project_id());
        let resp = issues_time_in_status_breakdown(State(handles.state), Query(q))
            .await
            .expect("ok")
            .0;
        assert_eq!(resp.issue_count, 0);
        // Default project has 5 statuses (open, in-progress, closed,
        // dropped, failed); each segment is present with mean=0.
        assert_eq!(resp.status_segments.len(), 5);
        for seg in &resp.status_segments {
            assert_eq!(seg.mean_seconds, 0);
        }
    }
}
