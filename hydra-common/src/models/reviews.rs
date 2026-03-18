use crate::HydraId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReviewDraft {
    /// Identifier for the review from Octocrab `Review.id`.
    pub review_id: HydraId,
    /// Reviewer login from Octocrab `Review.user.login`.
    pub author: String,
    /// Review state from Octocrab `Review.state` (ex: approved, changes_requested, commented).
    pub review_state: String,
    /// Timestamp when the review was submitted from Octocrab `Review.submitted_at`.
    pub submitted_at: Option<DateTime<Utc>>,
    /// Top-level review body from Octocrab `Review.body`.
    pub review_message: Option<String>,
    /// Inline review comments associated with this review.
    pub comments: Vec<ReviewCommentDraft>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReviewCommentDraft {
    /// Identifier for the comment from Octocrab `ReviewComment.id`.
    pub comment_id: HydraId,
    /// Identifier of the parent review from Octocrab `ReviewComment.pull_request_review_id`.
    pub review_id: HydraId,
    /// Comment body text from Octocrab `ReviewComment.body`.
    pub body: String,
    /// API URL or HTML URL from Octocrab `ReviewComment.url`/`ReviewComment.html_url`.
    pub url: Option<String>,
    /// File path that the comment applies to from Octocrab `ReviewComment.path`.
    pub filepath: Option<String>,
    /// Starting line number for the commented range from Octocrab `ReviewComment.start_line`.
    pub start_line: Option<u32>,
    /// Ending line number for the commented range from Octocrab `ReviewComment.line`.
    pub end_line: Option<u32>,
    /// Identifier for the parent comment from Octocrab `ReviewComment.in_reply_to_id`.
    pub in_reply_to: Option<HydraId>,
    /// Timestamp for comment creation from Octocrab `ReviewComment.created_at`.
    pub created_at: Option<DateTime<Utc>>,
    /// Timestamp for comment update from Octocrab `ReviewComment.updated_at`.
    pub updated_at: Option<DateTime<Utc>>,
}
