//! One-shot data migrations executed once at server startup.
//!
//! Unlike [`crate::migration_tool`] (an out-of-process CLI that operates on
//! raw SQL pools), the passes here run in-process against the `Store` trait
//! object owned by `AppState`, after the storage layer's schema migrations
//! complete and before the background scheduler / automation runner spin up.
//! Each pass must be idempotent so successive restarts are no-ops.

pub mod synthesise_merge_policy;
