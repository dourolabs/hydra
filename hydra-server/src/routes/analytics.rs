//! Route handlers for `/v1/analytics/throughput/patches/...`.
//!
//! Each handler validates the time window, fetches matching patches
//! and their histories via the store primitives, then delegates to the
//! pure aggregator in [`crate::analytics`]. The aggregator
//! holds the entire business logic; the handlers are a thin IO shim
//! plus error mapping.

use crate::analytics::{
    CostPerAgentAccumulator, IssuesCycleTimeAccumulator, IssuesOverTimeAccumulator,
    IssuesPerStatusDistributionAccumulator, IssuesTimeInStatusBreakdownAccumulator,
    PatchesInFlightOverTimeAccumulator, PatchesOverTimeAccumulator, PatchesTerminalMixAccumulator,
    PatchesTimeToMergeAccumulator, TokenUsageOverTimeAccumulator, TopIssuesByCostAccumulator,
    compute_top_issues_by_cost, for_each_issue_history, for_each_patch_history,
    for_each_session_with_usage,
};
use crate::app::AppState;
use crate::app::projects::{ResolveStatusError, project_cached};
use crate::store::StoreError;
use anyhow::anyhow;
use axum::{
    Json,
    extract::{Query, State},
};
use hydra_common::api::v1::{
    ApiError,
    analytics::{
        BucketGranularity, IssuesCycleTimeResponse, IssuesOverTimeResponse,
        IssuesPerStatusDistributionResponse, IssuesThroughputQuery,
        IssuesTimeInStatusBreakdownResponse, PatchesInFlightOverTimeResponse,
        PatchesOverTimeResponse, PatchesTerminalMixResponse, PatchesThroughputQuery,
        PatchesTimeToMergeResponse, TokenUsageCostPerAgentResponse, TokenUsageOverTimeQuery,
        TokenUsageOverTimeResponse, TokenUsageQuery, TokenUsageTopIssuesByCostResponse,
    },
    projects::Project,
};
use hydra_common::{IssueId, ProjectId};
use std::collections::HashMap;
use tracing::{error, info, warn};

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
    let mut acc = PatchesOverTimeAccumulator::new(query.from, query.to, bucket);
    for_each_patch_history(state.store(), &query, |h| acc.fold(h))
        .await
        .map_err(map_store_error)?;
    Ok(Json(acc.finalize()))
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
    let mut acc = PatchesTerminalMixAccumulator::new(query.from, query.to);
    for_each_patch_history(state.store(), &query, |h| acc.fold(h))
        .await
        .map_err(map_store_error)?;
    Ok(Json(acc.finalize()))
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
    let mut acc = PatchesTimeToMergeAccumulator::new(query.from, query.to);
    for_each_patch_history(state.store(), &query, |h| acc.fold(h))
        .await
        .map_err(map_store_error)?;
    Ok(Json(acc.finalize()))
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
    let mut acc = PatchesInFlightOverTimeAccumulator::new(query.from, query.to, bucket);
    for_each_patch_history(state.store(), &query, |h| acc.fold(h))
        .await
        .map_err(map_store_error)?;
    Ok(Json(acc.finalize()))
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
    let mut acc = IssuesCycleTimeAccumulator::new(query.from, query.to, query.status_keys.clone());
    let mut cache: HashMap<ProjectId, Project> = HashMap::new();
    for_each_issue_history(state.store(), &query, &mut cache, |h, p| acc.fold(h, p))
        .await
        .map_err(map_store_error)?;
    Ok(Json(acc.finalize()))
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
    let mut acc =
        IssuesOverTimeAccumulator::new(query.from, query.to, bucket, query.status_keys.clone());
    let mut cache: HashMap<ProjectId, Project> = HashMap::new();
    for_each_issue_history(state.store(), &query, &mut cache, |h, p| acc.fold(h, p))
        .await
        .map_err(map_store_error)?;
    Ok(Json(acc.finalize()))
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
    let mut acc = IssuesTimeInStatusBreakdownAccumulator::new(
        project_id.clone(),
        &project,
        query.from,
        query.to,
        query.status_keys.clone(),
    );
    // Seed the cache so `for_each_issue_history` skips the redundant
    // per-issue project lookup; the SearchIssuesQuery filter already
    // restricts results to this project_id.
    let mut cache: HashMap<ProjectId, Project> = HashMap::new();
    cache.insert(project_id.clone(), project.clone());
    for_each_issue_history(state.store(), &query, &mut cache, |h, _p| acc.fold(h))
        .await
        .map_err(map_store_error)?;
    Ok(Json(acc.finalize()))
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
    let mut acc = IssuesPerStatusDistributionAccumulator::new(
        project_id.clone(),
        &project,
        query.from,
        query.to,
        query.status_keys.clone(),
    );
    // Seed the cache so `for_each_issue_history` skips the redundant
    // per-issue project lookup; the SearchIssuesQuery filter already
    // restricts results to this project_id.
    let mut cache: HashMap<ProjectId, Project> = HashMap::new();
    cache.insert(project_id.clone(), project.clone());
    for_each_issue_history(state.store(), &query, &mut cache, |h, _p| acc.fold(h))
        .await
        .map_err(map_store_error)?;
    Ok(Json(acc.finalize()))
}

/// Validate the time window for the token-usage time-series query.
fn validate_token_usage_over_time_query(
    query: &TokenUsageOverTimeQuery,
) -> Result<BucketGranularity, ApiError> {
    if query.from >= query.to {
        return Err(ApiError::bad_request("'from' must be strictly before 'to'"));
    }
    Ok(query.bucket.unwrap_or_default())
}

/// Validate the time window for the non-time-series token-usage queries.
fn validate_token_usage_query(query: &TokenUsageQuery) -> Result<(), ApiError> {
    if query.from >= query.to {
        return Err(ApiError::bad_request("'from' must be strictly before 'to'"));
    }
    Ok(())
}

/// `GET /v1/analytics/token_usage/over_time`
pub async fn token_usage_over_time(
    State(state): State<AppState>,
    Query(query): Query<TokenUsageOverTimeQuery>,
) -> Result<Json<TokenUsageOverTimeResponse>, ApiError> {
    info!(
        from = %query.from,
        to = %query.to,
        bucket = ?query.bucket,
        "analytics.token_usage_over_time invoked"
    );
    let bucket = validate_token_usage_over_time_query(&query)?;
    let mut acc = TokenUsageOverTimeAccumulator::new(query.from, query.to, bucket);
    for_each_session_with_usage(
        state.store(),
        query.from,
        query.to,
        query.repo_name.as_deref(),
        query.creator.as_deref(),
        |s| acc.fold(s),
    )
    .await
    .map_err(map_store_error)?;
    Ok(Json(acc.finalize()))
}

/// `GET /v1/analytics/token_usage/cost_per_agent`
pub async fn token_usage_cost_per_agent(
    State(state): State<AppState>,
    Query(query): Query<TokenUsageQuery>,
) -> Result<Json<TokenUsageCostPerAgentResponse>, ApiError> {
    info!(
        from = %query.from,
        to = %query.to,
        "analytics.token_usage_cost_per_agent invoked"
    );
    validate_token_usage_query(&query)?;
    let mut acc = CostPerAgentAccumulator::new();
    for_each_session_with_usage(
        state.store(),
        query.from,
        query.to,
        query.repo_name.as_deref(),
        query.creator.as_deref(),
        |s| acc.fold(s),
    )
    .await
    .map_err(map_store_error)?;
    Ok(Json(acc.finalize()))
}

/// `GET /v1/analytics/token_usage/top_issues_by_cost`
pub async fn token_usage_top_issues_by_cost(
    State(state): State<AppState>,
    Query(query): Query<TokenUsageQuery>,
) -> Result<Json<TokenUsageTopIssuesByCostResponse>, ApiError> {
    info!(
        from = %query.from,
        to = %query.to,
        "analytics.token_usage_top_issues_by_cost invoked"
    );
    validate_token_usage_query(&query)?;
    let mut acc = TopIssuesByCostAccumulator::new();
    for_each_session_with_usage(
        state.store(),
        query.from,
        query.to,
        query.repo_name.as_deref(),
        query.creator.as_deref(),
        |s| acc.fold(s),
    )
    .await
    .map_err(map_store_error)?;
    let ranked = acc.finalize();
    let mut titles: HashMap<IssueId, String> = HashMap::with_capacity(ranked.len());
    for (issue_id, _, _) in &ranked {
        match state.store().get_issue(issue_id, false).await {
            Ok(versioned) => {
                titles.insert(issue_id.clone(), versioned.item.title);
            }
            Err(StoreError::IssueNotFound(_)) => {
                warn!(issue_id = %issue_id, "top_issues_by_cost: spawning issue not found; dropping");
            }
            Err(err) => return Err(map_store_error(err)),
        }
    }
    let resp = compute_top_issues_by_cost(ranked, &titles);
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

    fn smoke_token_usage_over_time_query() -> TokenUsageOverTimeQuery {
        TokenUsageOverTimeQuery::new(dt("2026-05-10T00:00:00Z"), dt("2026-05-13T00:00:00Z"))
    }

    fn smoke_token_usage_query() -> TokenUsageQuery {
        TokenUsageQuery::new(dt("2026-05-10T00:00:00Z"), dt("2026-05-13T00:00:00Z"))
    }

    #[tokio::test]
    async fn token_usage_over_time_with_empty_store_is_dense_zero_series() {
        let handles = test_state_handles();
        let resp = token_usage_over_time(
            State(handles.state),
            Query(smoke_token_usage_over_time_query()),
        )
        .await
        .expect("ok")
        .0;
        assert_eq!(resp.buckets.len(), 3);
        for b in &resp.buckets {
            assert_eq!(b.input_tokens, 0);
            assert_eq!(b.output_tokens, 0);
            assert_eq!(b.cache_read_input_tokens, 0);
            assert_eq!(b.cache_creation_input_tokens, 0);
        }
    }

    #[tokio::test]
    async fn token_usage_over_time_inverted_window_returns_400() {
        let handles = test_state_handles();
        let mut q = smoke_token_usage_over_time_query();
        q.to = q.from;
        let err = token_usage_over_time(State(handles.state), Query(q))
            .await
            .expect_err("inverted window must 400");
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn token_usage_cost_per_agent_with_empty_store_returns_no_agents() {
        let handles = test_state_handles();
        let resp =
            token_usage_cost_per_agent(State(handles.state), Query(smoke_token_usage_query()))
                .await
                .expect("ok")
                .0;
        assert!(resp.agents.is_empty());
    }

    #[tokio::test]
    async fn token_usage_cost_per_agent_inverted_window_returns_400() {
        let handles = test_state_handles();
        let mut q = smoke_token_usage_query();
        q.to = q.from;
        let err = token_usage_cost_per_agent(State(handles.state), Query(q))
            .await
            .expect_err("inverted window must 400");
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn token_usage_top_issues_by_cost_with_empty_store_returns_no_issues() {
        let handles = test_state_handles();
        let resp =
            token_usage_top_issues_by_cost(State(handles.state), Query(smoke_token_usage_query()))
                .await
                .expect("ok")
                .0;
        assert!(resp.issues.is_empty());
    }

    #[tokio::test]
    async fn token_usage_top_issues_by_cost_inverted_window_returns_400() {
        let handles = test_state_handles();
        let mut q = smoke_token_usage_query();
        q.to = q.from;
        let err = token_usage_top_issues_by_cost(State(handles.state), Query(q))
            .await
            .expect_err("inverted window must 400");
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    /// Seed > ANALYTICS_BATCH_SIZE sessions and assert the handler's
    /// totals match a single-batch computation. Forces the batched
    /// driver to advance the cursor at least once and exercises the
    /// token_usage_over_time accumulator across the boundary end-to-end.
    #[tokio::test]
    async fn token_usage_over_time_handler_matches_across_batch_boundary() {
        use crate::analytics::ANALYTICS_BATCH_SIZE;
        use crate::domain::sessions::{AgentConfig, Session, SessionMode};
        use crate::domain::task_status::Status;
        use crate::domain::users::Username;
        use crate::routes::sessions::mount_spec_from_create_request;
        use hydra_common::ActorRef as CommonActorRef;
        use hydra_common::api::v1::agents::AgentName;
        use hydra_common::api::v1::sessions::{Bundle, TokenUsage};

        let handles = test_state_handles();
        let store = handles.store.clone();
        let actor = CommonActorRef::test();

        let total = (ANALYTICS_BATCH_SIZE + 25) as usize;
        let mut total_input: u64 = 0;
        for i in 0..total {
            let agent_config = AgentConfig {
                agent_name: Some(AgentName::try_new("swe").expect("valid agent name")),
                model: None,
                system_prompt: None,
                mcp_config: None,
            };
            // Spread end_time across distinct seconds so the cursor
            // ordering is deterministic and every session lands in the
            // window.
            let end = dt("2026-05-10T00:00:00Z") + chrono::Duration::seconds(i as i64);
            let mut session = Session::new(
                Username::from("test"),
                None,
                None,
                agent_config,
                mount_spec_from_create_request(Bundle::None, None),
                None,
                std::collections::HashMap::new(),
                None,
                None,
                None,
                SessionMode::Headless,
                Status::Complete,
                None,
                None,
            );
            session.end_time = Some(end);
            session.creation_time = Some(end);
            let input = (i as u64 + 1) * 100;
            total_input += input;
            session.usage = Some(TokenUsage {
                input_tokens: input,
                output_tokens: 0,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            });
            store
                .add_session(session, end, &actor)
                .await
                .expect("add session");
        }

        let from = dt("2026-05-09T00:00:00Z");
        let to = dt("2026-05-13T00:00:00Z");
        let q = TokenUsageOverTimeQuery::new(from, to);
        let resp = token_usage_over_time(State(handles.state), Query(q))
            .await
            .expect("handler ok")
            .0;
        let observed_input: u64 = resp.buckets.iter().map(|b| b.input_tokens).sum();
        assert_eq!(
            observed_input, total_input,
            "every seeded session must contribute exactly once across the batched sweep"
        );
    }

    /// Seed > ANALYTICS_BATCH_SIZE patches and assert the handler's
    /// totals match a single-batch computation. Forces the batched
    /// driver to advance the cursor at least once and exercises the
    /// over_time accumulator across the boundary end-to-end. This is
    /// the regression bar called out in [[i-rqquesth]].
    #[tokio::test]
    async fn patches_over_time_handler_matches_across_batch_boundary() {
        use crate::analytics::ANALYTICS_BATCH_SIZE;
        use crate::domain::patches::{Patch, PatchStatus};
        use crate::domain::users::Username;
        use hydra_common::ActorRef as CommonActorRef;
        use hydra_common::RepoName;

        let handles = test_state_handles();
        let store = handles.store.clone();
        let actor = CommonActorRef::test();
        let repo_a = RepoName::new("dourolabs", "hydra").expect("repo name");

        let total = (ANALYTICS_BATCH_SIZE + 25) as usize;
        for _ in 0..total {
            let p = Patch::new(
                "title".to_string(),
                "desc".to_string(),
                "diff".to_string(),
                PatchStatus::Open,
                false,
                Username::from("alice"),
                Vec::new(),
                repo_a.clone(),
                None,
                None,
                None,
                None,
            );
            store.add_patch(p, &actor).await.expect("add patch");
        }

        // Window starts well before "now" and extends past it so every
        // created-now patch lands in the final bucket.
        let from = dt("2020-01-01T00:00:00Z");
        let to = dt("2100-01-01T00:00:00Z");
        let q = PatchesThroughputQuery::new(from, to);
        let resp = patches_over_time(State(handles.state), Query(q))
            .await
            .expect("handler ok")
            .0;
        let total_created: u64 = resp.buckets.iter().map(|b| b.created).sum();
        assert_eq!(
            total_created, total as u64,
            "every seeded patch must show up exactly once across the batched sweep"
        );
        let total_merged: u64 = resp.buckets.iter().map(|b| b.merged).sum();
        assert_eq!(total_merged, 0);
    }

    /// Seed > ANALYTICS_BATCH_SIZE issues spanning multiple projects and
    /// assert the `issues_over_time` handler totals match what every
    /// seeded issue would contribute. Forces the batched issue driver to
    /// advance the cursor at least once and exercises the over_time
    /// accumulator + project cache across the boundary end-to-end. This
    /// is the regression bar called out in [[i-ioalwzhs]].
    #[tokio::test]
    async fn issues_over_time_handler_matches_across_batch_boundary() {
        use crate::analytics::ANALYTICS_BATCH_SIZE;
        use crate::domain::issues::{Issue as DomainIssue, IssueType as DomainIssueType};
        use crate::domain::projects::{default_project_id, default_project_seed};
        use crate::domain::users::Username;
        use hydra_common::ActorRef as CommonActorRef;
        use hydra_common::api::v1::projects::{ProjectKey, StatusKey};

        let handles = test_state_handles();
        let store = handles.store.clone();
        let actor = CommonActorRef::test();

        // Seed a second project so the streaming driver has to resolve
        // more than one project across the batched sweep. Reuse the
        // default seed's status set, just change the key/name so the
        // project store accepts it as a distinct row. `add_project`
        // persists the project metadata; statuses must be added
        // separately via `add_status` to land in the project's status
        // index, so the cloned statuses are walked into place after the
        // project row is created.
        let mut alt_project = default_project_seed();
        alt_project.key = ProjectKey::try_new("alt").expect("alt project key");
        alt_project.name = "Alt Project".to_string();
        let alt_statuses = alt_project.statuses.clone();
        let (alt_project_id, _) = store
            .add_project(alt_project, &actor)
            .await
            .expect("add alt project");
        for status in alt_statuses {
            store
                .add_status(&alt_project_id, status, &actor)
                .await
                .expect("add alt status");
        }

        let default_pid = default_project_id();
        let total = (ANALYTICS_BATCH_SIZE + 25) as usize;
        for i in 0..total {
            // Alternate between the default project and the alt project
            // so the driver has to resolve both inside the loop and the
            // cache holds entries from both projects across batches.
            let project_id = if i % 2 == 0 {
                default_pid.clone()
            } else {
                alt_project_id.clone()
            };
            let issue = DomainIssue::new(
                DomainIssueType::Task,
                "title".to_string(),
                "desc".to_string(),
                Username::from("alice"),
                StatusKey::try_new("open").expect("status key"),
                project_id,
                None,
                None,
                Vec::new(),
                Vec::new(),
                None,
                None,
            );
            store.add_issue(issue, &actor).await.expect("add issue");
        }

        // Wide window comfortably containing today's `Utc::now()` so the
        // streaming sweep visits every seeded issue.
        let from = dt("2020-01-01T00:00:00Z");
        let to = dt("2100-01-01T00:00:00Z");
        let q = IssuesThroughputQuery::new(from, to);
        let resp = issues_over_time(State(handles.state), Query(q))
            .await
            .expect("handler ok")
            .0;
        let total_created: u64 = resp.buckets.iter().map(|b| b.created).sum();
        assert_eq!(
            total_created, total as u64,
            "every seeded issue must show up exactly once across the batched sweep"
        );
        // None of the seeded issues reached a terminal status.
        let total_terminal: u64 = resp.buckets.iter().map(|b| b.reached_terminal).sum();
        assert_eq!(total_terminal, 0);
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
