//! Build [`api_issues::Issue`] responses with the server-computed
//! `resolved_status` field populated.
//!
//! Centralizing the resolution here (rather than duplicating it per
//! route) keeps the contract from `/designs/per-project-issue-statuses.md`
//! §4 "Frontend display" honored consistently: every `Issue` returned
//! over the wire carries its `resolved_status` inline.

use crate::app::AppState;
use crate::app::projects::ResolveStatusError;
use crate::domain::issues::Issue as DomainIssue;
use anyhow::anyhow;
use hydra_common::api::v1::{self as api, ApiError};
use tracing::error;

/// Convert a domain [`Issue`] into an [`api::issues::Issue`] response,
/// populating `resolved_status` via [`AppState::resolve_status`].
pub async fn build_issue_response(
    state: &AppState,
    issue: DomainIssue,
) -> Result<api::issues::Issue, ApiError> {
    let resolved = state
        .resolve_status(&issue)
        .await
        .map_err(map_resolve_error)?;
    let mut api_issue: api::issues::Issue = issue.into();
    api_issue.resolved_status = Some(resolved);
    Ok(api_issue)
}

/// Same as [`build_issue_response`] but operates on a [`DomainIssue`]
/// by reference, returning the constructed API issue. Useful for list
/// endpoints that need to keep the original around for summary mapping.
pub async fn build_issue_summary_response(
    state: &AppState,
    issue: &DomainIssue,
) -> Result<api::issues::IssueSummary, ApiError> {
    let resolved = state
        .resolve_status(issue)
        .await
        .map_err(map_resolve_error)?;
    let api_issue: api::issues::Issue = issue.clone().into();
    let mut summary = api::issues::IssueSummary::from(&api_issue);
    summary.resolved_status = Some(resolved);
    Ok(summary)
}

fn map_resolve_error(err: ResolveStatusError) -> ApiError {
    match err {
        ResolveStatusError::InvalidKey(_) | ResolveStatusError::UnknownStatus(_) => {
            // Validation should reject before reaching here; surface
            // as a server-side inconsistency so it's visible in logs.
            error!(error = %err, "failed to resolve status for issue response");
            ApiError::internal(anyhow!("failed to resolve issue status: {err}"))
        }
        ResolveStatusError::ProjectNotFound(_) => {
            error!(error = %err, "referenced project missing during status resolution");
            ApiError::internal(anyhow!("failed to resolve issue status: {err}"))
        }
        ResolveStatusError::Store(_) => {
            error!(error = %err, "store error during status resolution");
            ApiError::internal(anyhow!("failed to resolve issue status: {err}"))
        }
    }
}
