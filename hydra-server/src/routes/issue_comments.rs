//! HTTP routes for per-issue comments. Comments are append-only and
//! addressed by `(issue_id, sequence)`. See
//! `hydra-common/src/api/v1/comments.rs` for the wire shapes.

use anyhow::anyhow;
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use hydra_common::{
    IssueId,
    api::v1::{
        ApiError,
        comments::{
            AddCommentRequest, AddCommentResponse, ListCommentsQuery, ListCommentsResponse,
        },
    },
};
use tracing::{error, info};

use crate::app::AppState;
use crate::domain::actors::{Actor, ActorRef};
use crate::routes::issues::IssueIdPath;
use crate::store::StoreError;

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

pub async fn add_issue_comment(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    IssueIdPath(issue_id): IssueIdPath,
    Json(request): Json<AddCommentRequest>,
) -> Result<Json<AddCommentResponse>, ApiError> {
    info!(actor = %actor.name(), issue_id = %issue_id, "add_issue_comment invoked");

    let body = request.body;
    if body.trim().is_empty() {
        return Err(ApiError::bad_request(
            "comment body must not be empty or whitespace-only",
        ));
    }

    let actor_ref = ActorRef::from(&actor);
    let comment = state
        .add_comment(&issue_id, body, &actor_ref)
        .await
        .map_err(|err| map_comment_error(err, &issue_id))?;

    info!(
        actor = %actor.name(),
        issue_id = %issue_id,
        sequence = comment.sequence,
        "add_issue_comment completed"
    );
    Ok(Json(AddCommentResponse::new(comment.into())))
}

pub async fn list_issue_comments(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    IssueIdPath(issue_id): IssueIdPath,
    Query(query): Query<ListCommentsQuery>,
) -> Result<Json<ListCommentsResponse>, ApiError> {
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    info!(
        actor = %actor.name(),
        issue_id = %issue_id,
        limit,
        before_sequence = ?query.before_sequence,
        "list_issue_comments invoked"
    );

    let page = state
        .list_comments(&issue_id, limit, query.before_sequence)
        .await
        .map_err(|err| map_comment_error(err, &issue_id))?;

    let count = page.comments.len();
    info!(
        actor = %actor.name(),
        issue_id = %issue_id,
        count,
        "list_issue_comments completed"
    );
    Ok(Json(page.into()))
}

fn map_comment_error(err: StoreError, issue_id: &IssueId) -> ApiError {
    match err {
        StoreError::IssueNotFound(id) => ApiError::not_found(format!("issue '{id}' not found")),
        other => {
            error!(
                issue_id = %issue_id,
                error = %other,
                "comment store operation failed"
            );
            ApiError::internal(anyhow!("comment store error: {other}"))
        }
    }
}
