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
//! [`StatusDefinition`]. `closed`, `dropped`, and `failed` are all
//! terminal under this definition; clients that want to exclude the
//! cancellation lanes can pass `status_keys=closed` on the query.
//!
//! [`StatusDefinition`]: hydra_common::api::v1::projects::StatusDefinition

mod buckets;
mod issues;
mod patches;
mod pricing;
mod token_usage;

/// Batch size used when streaming patches / issues / sessions through
/// the analytics aggregators. Matches
/// [`hydra_common::api::v1::pagination::MAX_LIMIT`] — the same ceiling
/// every other paginated route applies — so callers pay one round-trip
/// per 200 rows instead of materializing the full list.
pub const ANALYTICS_BATCH_SIZE: u32 = 200;

pub use issues::{
    IssueHistory, IssuesCycleTimeAccumulator, IssuesOverTimeAccumulator,
    IssuesPerStatusDistributionAccumulator, IssuesTimeInStatusBreakdownAccumulator,
    compute_issues_cycle_time, compute_issues_over_time, compute_issues_per_status_distribution,
    compute_issues_time_in_status_breakdown, for_each_issue_history,
};
pub use patches::{
    PatchHistory, PatchesInFlightOverTimeAccumulator, PatchesOverTimeAccumulator,
    PatchesTerminalMixAccumulator, PatchesTimeToMergeAccumulator,
    compute_patches_in_flight_over_time, compute_patches_over_time, compute_patches_terminal_mix,
    compute_patches_time_to_merge, for_each_patch_history,
};
pub use token_usage::{
    CostPerAgentAccumulator, SessionWithUsage, TokenUsageOverTimeAccumulator,
    TopIssuesByCostAccumulator, compute_cost_per_agent, compute_token_usage_over_time,
    compute_top_issues_by_cost, for_each_session_with_usage, rank_top_issues_by_cost,
};
