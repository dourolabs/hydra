//! Domain type for per-issue comments. Mirrors the wire shape in
//! [`hydra_common::api::v1::comments`]; the conversion is structural
//! today and lives here so the store layer can hand domain `Comment`
//! values back to the route layer without re-deriving the wire shape.

use chrono::{DateTime, Utc};

use hydra_common::IssueId;
use hydra_common::actor_ref::ActorRef;
use hydra_common::api::v1::comments as wire;

#[derive(Debug, Clone, PartialEq, Eq)]
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

impl From<Comment> for wire::Comment {
    fn from(comment: Comment) -> Self {
        wire::Comment::new(
            comment.issue_id,
            comment.sequence,
            comment.body,
            comment.actor,
            comment.created_at,
        )
    }
}

/// A single page returned by `Store::list_comments`. Comments are
/// ordered most-recent-first; `next_before_sequence` is `Some` only
/// when the page is full and a further cursor is available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListCommentsPage {
    pub comments: Vec<Comment>,
    pub next_before_sequence: Option<u64>,
}

impl ListCommentsPage {
    pub fn new(comments: Vec<Comment>, next_before_sequence: Option<u64>) -> Self {
        Self {
            comments,
            next_before_sequence,
        }
    }
}

impl From<ListCommentsPage> for wire::ListCommentsResponse {
    fn from(page: ListCommentsPage) -> Self {
        wire::ListCommentsResponse::new(
            page.comments.into_iter().map(Into::into).collect(),
            page.next_before_sequence,
        )
    }
}
