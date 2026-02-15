use metis_common::{VersionNumber, api::v1::ApiError};

pub mod agents;
pub mod auth;
pub mod documents;
pub mod events;
pub mod github;
pub mod issues;
pub mod jobs;
pub mod login;
pub mod merge_queues;
pub mod patches;
pub mod repositories;
pub mod users;
pub mod whoami;

/// Resolve a version number that may be negative (offset from latest) into an
/// absolute positive version number.
///
/// - Positive values are returned as-is.
/// - Negative values are treated as offsets from `max_version` (e.g. -1 means
///   the second-to-last version).
/// - Zero is rejected with 400 Bad Request.
/// - Out-of-range negative offsets (resolving to < 1) are rejected with 400.
pub(crate) fn resolve_version(
    version: i64,
    max_version: VersionNumber,
    entity_label: &str,
    entity_id: &str,
) -> Result<VersionNumber, ApiError> {
    if version == 0 {
        return Err(ApiError::bad_request(
            "version 0 is not valid; use a positive version number or a negative offset from the latest version",
        ));
    }

    if version > 0 {
        return Ok(version as VersionNumber);
    }

    // version is negative — resolve relative to max_version
    let target = max_version as i64 + version;
    if target < 1 {
        return Err(ApiError::bad_request(format!(
            "version offset {version} is out of range for {entity_label} '{entity_id}' \
             which has {max_version} version(s)",
        )));
    }
    Ok(target as VersionNumber)
}
