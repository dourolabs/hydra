use chrono::{DateTime, Utc};

use crate::domain::patches::{Patch, Review};
use metis_common::versioning::Versioned;

/// Finds the latest non-stale review by a given author.
///
/// Converts domain `Review` types to API types and delegates to the shared
/// implementation in `metis_common::review_utils`.
pub fn find_latest_review_by_author(
    reviews: &[Review],
    author: &str,
    staleness_cutoff: Option<DateTime<Utc>>,
) -> Option<metis_common::api::v1::patches::Review> {
    let api_reviews: Vec<metis_common::api::v1::patches::Review> =
        reviews.iter().cloned().map(Into::into).collect();
    metis_common::review_utils::find_latest_review_by_author(&api_reviews, author, staleness_cutoff)
        .cloned()
}

/// Finds the timestamp of the last version where the patch's `commit_range` changed.
///
/// Converts domain `Versioned<Patch>` to `PatchVersionRecord` and delegates to the
/// shared implementation in `metis_common::review_utils`.
pub fn find_last_commit_range_change_timestamp(
    versions: &[Versioned<Patch>],
) -> Option<DateTime<Utc>> {
    // Use a dummy patch_id; the shared function only inspects commit_range and timestamp.
    let dummy_patch_id = metis_common::PatchId::new();
    let api_versions: Vec<metis_common::api::v1::patches::PatchVersionRecord> = versions
        .iter()
        .map(|v| {
            metis_common::api::v1::patches::PatchVersionRecord::new(
                dummy_patch_id.clone(),
                v.version,
                v.timestamp,
                v.item.clone().into(),
                v.actor.clone(),
                v.creation_time,
                Vec::new(),
            )
        })
        .collect();
    metis_common::review_utils::find_last_commit_range_change_timestamp(&api_versions)
}

/// Returns `true` if there is at least one approved, non-dismissed (non-stale)
/// review on the patch, considering the version history for staleness.
///
/// Converts domain types to API types and delegates to the shared
/// implementation in `metis_common::review_utils`.
pub fn has_approved_non_dismissed_review(
    reviews: &[Review],
    staleness_cutoff: Option<DateTime<Utc>>,
) -> bool {
    let api_reviews: Vec<metis_common::api::v1::patches::Review> =
        reviews.iter().cloned().map(Into::into).collect();
    metis_common::review_utils::has_approved_non_dismissed_review(&api_reviews, staleness_cutoff)
}
