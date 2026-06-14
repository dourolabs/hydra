//! Thin AppState pass-throughs for the comments store surface. Comments
//! are append-only and do not bump the parent issue's version, but
//! `add_comment` does emit a `comment_created` event on the event bus
//! so SSE subscribers can react. The wrappers delegate to the
//! underlying store for the persistence step.

use crate::domain::actors::ActorRef;
use crate::domain::comments::{Comment, ListCommentsPage};
use crate::store::{ReadOnlyStore, StoreError};
use hydra_common::IssueId;

use super::AppState;

impl AppState {
    pub async fn add_comment(
        &self,
        issue_id: &IssueId,
        body: String,
        actor: &ActorRef,
    ) -> Result<Comment, StoreError> {
        self.store.add_comment(issue_id, body, actor).await
    }

    pub async fn list_comments(
        &self,
        issue_id: &IssueId,
        limit: u32,
        before_sequence: Option<u64>,
    ) -> Result<ListCommentsPage, StoreError> {
        self.store
            .list_comments(issue_id, limit, before_sequence)
            .await
    }
}
