use crate::domain::{actors::Actor, users::Username};
use metis_common::{RelativeVersionNumber, VersionNumber, api::v1::ApiError};

pub mod agents;
pub mod auth;
pub mod documents;
pub mod events;
#[cfg(feature = "github")]
pub mod github;
pub mod issues;
pub mod jobs;
pub mod labels;
#[cfg(feature = "github")]
pub mod login;
pub mod merge_queues;
pub mod messages;
pub mod notifications;
pub mod patches;
pub mod repositories;
pub mod secrets;
pub mod users;
pub mod whoami;

/// Resolve the `:username` path parameter: "me" maps to the authenticated
/// user's username; any other value is returned as-is.
pub(crate) fn resolve_username(actor: &Actor, raw: &str) -> Result<Username, ApiError> {
    if raw == "me" {
        Ok(actor.creator.clone())
    } else {
        Ok(Username::from(raw.to_string()))
    }
}

/// Resolve a version number that may be negative (offset from latest) into an
/// absolute positive version number.
///
/// - Positive values are returned as-is.
/// - Negative values are treated as offsets from `max_version` (e.g. -1 means
///   the second-to-last version).
/// - Zero is rejected with 400 Bad Request.
/// - Out-of-range negative offsets (resolving to < 1) are rejected with 400.
pub(crate) fn resolve_version(
    version: RelativeVersionNumber,
    max_version: VersionNumber,
    entity_label: &str,
    entity_id: &str,
) -> Result<VersionNumber, ApiError> {
    let version = version.as_i64();

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
