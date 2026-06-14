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
use hydra_common::api::v1::{
    self as api, ApiError,
    projects::{StatusDefinition, StatusKey},
};
use tracing::{error, warn};

/// Convert a domain [`DomainIssue`] into an [`api::issues::Issue`]
/// response, resolving its status against the project via
/// [`AppState::resolve_status`].
///
/// `ResolveStatusError::ProjectNotFound` and `UnknownStatus` are
/// treated as recoverable: the response falls back to a placeholder
/// [`StatusDefinition`] keyed on the issue's stored status, so that a
/// soft-deleted parent project or a missing status declaration does not
/// 500 the whole list/get response. All other variants are surfaced as
/// `ApiError::internal`.
pub async fn build_issue_response(
    state: &AppState,
    issue: DomainIssue,
) -> Result<api::issues::Issue, ApiError> {
    let resolved = match state.resolve_status(&issue).await {
        Ok(resolved) => resolved,
        Err(
            err @ (ResolveStatusError::ProjectNotFound(_) | ResolveStatusError::UnknownStatus(_)),
        ) => {
            warn!(
                error = %err,
                issue_status = %issue.status,
                project_id = %issue.project_id,
                "status resolution falling back to placeholder for unresolved parent",
            );
            placeholder_status(&issue.status)
        }
        Err(err) => return Err(map_resolve_error(err)),
    };
    Ok(api::issues::Issue::new(
        issue.issue_type.into(),
        issue.title,
        issue.description,
        issue.creator.into(),
        resolved,
        issue.project_id,
        issue.assignee,
        Some(issue.session_settings.into()),
        issue.dependencies.into_iter().map(Into::into).collect(),
        issue.patches,
        issue.deleted,
        issue.form,
        issue.form_response,
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

/// Synthesize a degraded [`StatusDefinition`] for an issue whose
/// project or status key can't be resolved. The frontend's
/// `UNRESOLVED_GROUP_KEY` fallback groups these rows by their (still
/// real) `project_id`, so the row remains visible in the list with the
/// stored status key preserved for display.
fn placeholder_status(key: &StatusKey) -> StatusDefinition {
    StatusDefinition::new(
        key.clone(),
        key.as_str().to_string(),
        "#808080"
            .parse()
            .expect("hard-coded gray placeholder is a valid Rgb"),
        false,
        false,
        false,
        None,
    )
}

fn map_resolve_error(err: ResolveStatusError) -> ApiError {
    match err {
        ResolveStatusError::InvalidKey(_) => {
            // Validation should reject before reaching here; surface
            // as a server-side inconsistency so it's visible in logs.
            error!(error = %err, "failed to resolve status for issue response");
            ApiError::internal(anyhow!("failed to resolve issue status: {err}"))
        }
        ResolveStatusError::Store(_) => {
            error!(error = %err, "store error during status resolution");
            ApiError::internal(anyhow!("failed to resolve issue status: {err}"))
        }
        // ProjectNotFound and UnknownStatus are handled in
        // `build_issue_response` via `placeholder_status`; they should
        // never reach this mapper. Keep them as 500s here as a guard.
        ResolveStatusError::ProjectNotFound(_) | ResolveStatusError::UnknownStatus(_) => {
            error!(error = %err, "unexpected resolve error reached error mapper");
            ApiError::internal(anyhow!("failed to resolve issue status: {err}"))
        }
    }
}
