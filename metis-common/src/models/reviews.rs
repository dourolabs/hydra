use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Review {
    /// Identifier for the review from Octocrab `Review.id`.
    #[serde(default)]
    pub review_id: u64,
    /// Reviewer login from Octocrab `Review.user.login`.
    #[serde(default)]
    pub author: String,
    /// Review state from Octocrab `Review.state` (ex: approved, changes_requested, commented).
    #[serde(default)]
    pub review_state: String,
    /// Timestamp when the review was submitted from Octocrab `Review.submitted_at`.
    pub submitted_at: Option<DateTime<Utc>>,
    /// Top-level review body from Octocrab `Review.body`.
    pub review_message: Option<String>,
    /// Inline review comments associated with this review.
    #[serde(default)]
    pub comments: Vec<Comment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Comment {
    /// Identifier for the comment from Octocrab `ReviewComment.id`.
    pub id: u64,
    /// Identifier of the parent review from Octocrab `ReviewComment.pull_request_review_id`.
    pub review_id: u64,
    /// Comment body text from Octocrab `ReviewComment.body`.
    pub text: String,
    /// API URL or HTML URL from Octocrab `ReviewComment.url`/`ReviewComment.html_url`.
    pub url: Option<String>,
    /// File path that the comment applies to from Octocrab `ReviewComment.path`.
    pub filepath: Option<String>,
    /// Starting line number for the commented range from Octocrab `ReviewComment.start_line`.
    pub start_line: Option<u32>,
    /// Ending line number for the commented range from Octocrab `ReviewComment.line`.
    pub end_line: Option<u32>,
    /// Identifier for the parent comment from Octocrab `ReviewComment.in_reply_to_id`.
    pub in_reply_to: Option<u64>,
    /// Timestamp for comment creation from Octocrab `ReviewComment.created_at`.
    pub created_at: Option<DateTime<Utc>>,
    /// Timestamp for comment update from Octocrab `ReviewComment.updated_at`.
    pub updated_at: Option<DateTime<Utc>>,
}

impl Review {
    pub fn new(
        review_id: u64,
        author: String,
        review_state: String,
        submitted_at: Option<DateTime<Utc>>,
        review_message: Option<String>,
        comments: Vec<Comment>,
    ) -> Self {
        Self {
            review_id,
            author,
            review_state,
            submitted_at,
            review_message,
            comments,
        }
    }
}

impl Comment {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: u64,
        review_id: u64,
        text: String,
        url: Option<String>,
        filepath: Option<String>,
        start_line: Option<u32>,
        end_line: Option<u32>,
        in_reply_to: Option<u64>,
        created_at: Option<DateTime<Utc>>,
        updated_at: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            id,
            review_id,
            text,
            url,
            filepath,
            start_line,
            end_line,
            in_reply_to,
            created_at,
            updated_at,
        }
    }
}
