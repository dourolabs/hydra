//! Wire types for per-issue comments.
//!
//! Comments are append-only, per-issue, and addressed by
//! `(issue_id, sequence)` where `sequence` starts at 1 and increments
//! per-issue. A new comment does NOT bump the issue's version and does
//! NOT wake the assigned agent.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::IssueId;
use crate::actor_ref::ActorRef;

/// A single comment on an issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct Comment {
    pub issue_id: IssueId,
    pub sequence: u64,
    pub body: String,
    pub actor: ActorRef,
    pub created_at: DateTime<Utc>,
}

impl Comment {
    pub fn new(
        issue_id: IssueId,
        sequence: u64,
        body: String,
        actor: ActorRef,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            issue_id,
            sequence,
            body,
            actor,
            created_at,
        }
    }
}

/// Request body for `POST /v1/issues/:issue_id/comments`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct AddCommentRequest {
    pub body: String,
}

impl AddCommentRequest {
    pub fn new(body: String) -> Self {
        Self { body }
    }
}

/// Response body for `POST /v1/issues/:issue_id/comments`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct AddCommentResponse {
    pub comment: Comment,
}

impl AddCommentResponse {
    pub fn new(comment: Comment) -> Self {
        Self { comment }
    }
}

/// Query parameters for `GET /v1/issues/:issue_id/comments`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListCommentsQuery {
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub before_sequence: Option<u64>,
}

/// Response body for `GET /v1/issues/:issue_id/comments`. Comments are
/// ordered most-recent-first (DESC by `sequence`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListCommentsResponse {
    pub comments: Vec<Comment>,
    /// Cursor for the next page (`?before_sequence=`). `None` when the
    /// returned batch was not full — i.e. there is no further page to
    /// fetch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_before_sequence: Option<u64>,
}

impl ListCommentsResponse {
    pub fn new(comments: Vec<Comment>, next_before_sequence: Option<u64>) -> Self {
        Self {
            comments,
            next_before_sequence,
        }
    }
}
