//! Build [`api::issues::Issue`] responses with the server-resolved
//! [`StatusDefinition`] inlined on the `status` field.
//!
//! Centralizing the resolution here (rather than duplicating it per
//! route) keeps the wire contract uniform: every `Issue` returned over
//! the wire carries its full status definition (label, color, flags)
//! inline, so the frontend can render badges without a second round-trip
//! and without re-implementing `(project_id, status_key) → StatusDefinition`
//! resolution on the client.

use crate::app::AppState;
use crate::app::projects::ResolveStatusError;
use crate::domain::issues::Issue as DomainIssue;
use anyhow::anyhow;
use hydra_common::api::v1::{self as api, ApiError};
use tracing::error;

/// Convert a domain [`DomainIssue`] into an [`api::issues::Issue`]
/// response, resolving its status against the project via
/// [`AppState::resolve_status`].
pub async fn build_issue_response(
    state: &AppState,
    issue: DomainIssue,
) -> Result<api::issues::Issue, ApiError> {
    let resolved = state
        .resolve_status(&issue)
        .await
        .map_err(map_resolve_error)?;
    Ok(api::issues::Issue::new(
        issue.issue_type.into(),
        issue.title,
        issue.description,
        issue.creator.into(),
        issue.progress,
        resolved,
        issue.project_id,
        issue.assignee,
        Some(issue.session_settings.into()),
        issue.dependencies.into_iter().map(Into::into).collect(),
        issue.patches,
        issue.deleted,
        issue.form,
        issue.form_response,
        issue.feedback,
    ))
}

/// Same as [`build_issue_response`] but operates on a [`DomainIssue`]
/// by reference, returning the constructed API summary. Useful for list
/// endpoints that need to keep the original around for other mapping.
pub async fn build_issue_summary_response(
    state: &AppState,
    issue: &DomainIssue,
) -> Result<api::issues::IssueSummary, ApiError> {
    let api_issue = build_issue_response(state, issue.clone()).await?;
    Ok(api::issues::IssueSummary::from(&api_issue))
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
