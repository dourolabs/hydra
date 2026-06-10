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
//!
//! [`StatusDefinition`]: hydra_common::api::v1::projects::StatusDefinition

mod buckets;
mod issues;
mod patches;

pub use issues::{
    IssueHistory, compute_issues_cycle_time, compute_issues_over_time,
    compute_issues_per_status_distribution, compute_issues_time_in_status_breakdown,
    fetch_issue_histories, resolve_projects_for_histories,
};
pub use patches::{
    PatchHistory, compute_patches_in_flight_over_time, compute_patches_over_time,
    compute_patches_terminal_mix, compute_patches_time_to_merge, fetch_patch_histories,
};
