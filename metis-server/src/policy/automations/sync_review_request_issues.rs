use async_trait::async_trait;

use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::actors::ActorRef;
use crate::domain::issues::{IssueStatus, IssueType};
use crate::domain::patches::Review;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};

const AUTOMATION_NAME: &str = "sync_review_request_issues";

/// Syncs ReviewRequest issue status with GitHub PR reviews.
///
/// When a PatchUpdated event fires and the patch has reviews:
/// - For each ReviewRequest issue linked to the patch that is still Open or InProgress:
///   - Find the latest review from the issue's assignee (case-insensitive match)
///   - If the latest review is approving, close the ReviewRequest issue
///   - If the latest review is non-approving, fail the ReviewRequest issue
/// - Issues already in a terminal status (Closed/Dropped/Failed) are skipped
pub struct SyncReviewRequestIssuesAutomation;

impl SyncReviewRequestIssuesAutomation {
    pub fn new(_params: Option<&toml::Value>) -> Result<Self, String> {
        Ok(Self)
    }
}

#[async_trait]
impl Automation for SyncReviewRequestIssuesAutomation {
    fn name(&self) -> &str {
        AUTOMATION_NAME
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![EventType::PatchUpdated],
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        let ServerEvent::PatchUpdated {
            patch_id, payload, ..
        } = ctx.event
        else {
            return Ok(());
        };

        let MutationPayload::Patch { new, .. } = payload.as_ref() else {
            return Ok(());
        };

        // Nothing to do if the patch has no reviews
        if new.reviews.is_empty() {
            return Ok(());
        }

        let store = ctx.store;
        let issue_ids = store.get_issues_for_patch(patch_id).await.map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to get issues for patch {patch_id}: {e}"
            ))
        })?;

        for issue_id in issue_ids {
            let issue = store.get_issue(&issue_id, false).await.map_err(|e| {
                AutomationError::Other(anyhow::anyhow!("failed to fetch issue {issue_id}: {e}"))
            })?;
            let mut issue = issue.item;

            // Only process ReviewRequest issues
            if issue.issue_type != IssueType::ReviewRequest {
                continue;
            }

            // Skip issues already in terminal status
            if matches!(
                issue.status,
                IssueStatus::Closed | IssueStatus::Dropped | IssueStatus::Failed
            ) {
                continue;
            }

            // Need an assignee to match against review authors
            let assignee = match &issue.assignee {
                Some(a) => a.clone(),
                None => continue,
            };

            // Find the latest review from the assignee (case-insensitive)
            let latest_review = find_latest_review_by_author(&new.reviews, &assignee);

            let Some(review) = latest_review else {
                continue;
            };

            let new_status = if review.is_approved {
                IssueStatus::Closed
            } else {
                IssueStatus::Failed
            };

            issue.status = new_status;

            ctx.app_state
                .upsert_issue(
                    Some(issue_id.clone()),
                    metis_common::api::v1::issues::UpsertIssueRequest::new(issue.into(), None),
                    ActorRef::Automation {
                        automation_name: AUTOMATION_NAME.into(),
                        triggered_by: Some(Box::new(ctx.actor().clone())),
                    },
                )
                .await
                .map_err(|e| {
                    AutomationError::Other(anyhow::anyhow!(
                        "failed to update review-request issue {issue_id}: {e}"
                    ))
                })?;

            tracing::info!(
                patch_id = %patch_id,
                issue_id = %issue_id,
                assignee = %assignee,
                new_status = %new_status,
                "synced review-request issue status from patch review"
            );
        }

        Ok(())
    }
}

/// Find the latest review by a given author (case-insensitive match).
/// When multiple reviews exist from the same author, the one with the latest
/// `submitted_at` timestamp wins. Reviews without a timestamp are treated
/// as older than any review with a timestamp.
fn find_latest_review_by_author<'a>(reviews: &'a [Review], author: &str) -> Option<&'a Review> {
    reviews
        .iter()
        .filter(|r| r.author.eq_ignore_ascii_case(author))
        .max_by(|a, b| {
            // Reviews with submitted_at are always newer than those without
            match (&a.submitted_at, &b.submitted_at) {
                (Some(a_time), Some(b_time)) => a_time.cmp(b_time),
                (Some(_), None) => std::cmp::Ordering::Greater,
                (None, Some(_)) => std::cmp::Ordering::Less,
                // If neither has a timestamp, use position (later in vec = newer)
                (None, None) => std::cmp::Ordering::Less,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::event_bus::MutationPayload;
    use crate::domain::actors::ActorRef;
    use crate::domain::issues::{Issue, IssueStatus, IssueType};
    use crate::domain::patches::{Patch, PatchStatus, Review};
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use crate::test_utils;
    use chrono::{Duration, Utc};
    use metis_common::RepoName;
    use std::sync::Arc;

    fn make_patch(status: PatchStatus, reviews: Vec<Review>) -> Patch {
        Patch::new(
            "test patch".to_string(),
            "desc".to_string(),
            String::new(),
            status,
            false,
            None,
            reviews,
            RepoName::new("test", "repo").unwrap(),
            None,
        )
    }

    fn make_review_request_issue(
        patch_id: &metis_common::PatchId,
        assignee: &str,
        status: IssueStatus,
    ) -> Issue {
        Issue::new(
            IssueType::ReviewRequest,
            format!("Review request for patch {}", patch_id.as_ref()),
            Username::from("tester"),
            String::new(),
            status,
            Some(assignee.to_string()),
            None,
            Vec::new(),
            Vec::new(),
            vec![patch_id.clone()],
        )
    }

    fn make_patch_updated_event(
        patch_id: metis_common::PatchId,
        old_patch: Patch,
        new_patch: Patch,
    ) -> ServerEvent {
        let payload = Arc::new(MutationPayload::Patch {
            old: Some(old_patch),
            new: new_patch,
            actor: ActorRef::test(),
        });
        ServerEvent::PatchUpdated {
            seq: 1,
            patch_id,
            version: 2,
            timestamp: Utc::now(),
            payload,
        }
    }

    #[tokio::test]
    async fn approving_review_closes_review_request() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let old_patch = make_patch(PatchStatus::Open, vec![]);
        let new_patch = make_patch(
            PatchStatus::Open,
            vec![Review::new(
                "LGTM".to_string(),
                true,
                "reviewer-a".to_string(),
                Some(Utc::now()),
            )],
        );
        let (patch_id, _) = store.add_patch(new_patch.clone()).await.unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue).await.unwrap();

        let event = make_patch_updated_event(patch_id, old_patch, new_patch);

        let automation = SyncReviewRequestIssuesAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let updated = store.get_issue(&rr_id, false).await.unwrap();
        assert_eq!(updated.item.status, IssueStatus::Closed);
    }

    #[tokio::test]
    async fn non_approving_review_fails_review_request() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let old_patch = make_patch(PatchStatus::Open, vec![]);
        let new_patch = make_patch(
            PatchStatus::Open,
            vec![Review::new(
                "Changes needed".to_string(),
                false,
                "reviewer-a".to_string(),
                Some(Utc::now()),
            )],
        );
        let (patch_id, _) = store.add_patch(new_patch.clone()).await.unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue).await.unwrap();

        let event = make_patch_updated_event(patch_id, old_patch, new_patch);

        let automation = SyncReviewRequestIssuesAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let updated = store.get_issue(&rr_id, false).await.unwrap();
        assert_eq!(updated.item.status, IssueStatus::Failed);
    }

    #[tokio::test]
    async fn skips_already_terminal_review_request() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let old_patch = make_patch(PatchStatus::Open, vec![]);
        let new_patch = make_patch(
            PatchStatus::Open,
            vec![Review::new(
                "LGTM".to_string(),
                true,
                "reviewer-a".to_string(),
                Some(Utc::now()),
            )],
        );
        let (patch_id, _) = store.add_patch(new_patch.clone()).await.unwrap();

        // Create ReviewRequest issues in various terminal statuses
        let closed_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Closed);
        let (closed_id, _) = store.add_issue(closed_issue).await.unwrap();

        let failed_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Failed);
        let (failed_id, _) = store.add_issue(failed_issue).await.unwrap();

        let dropped_issue =
            make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Dropped);
        let (dropped_id, _) = store.add_issue(dropped_issue).await.unwrap();

        let event = make_patch_updated_event(patch_id, old_patch, new_patch);

        let automation = SyncReviewRequestIssuesAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        // All should remain unchanged
        let closed = store.get_issue(&closed_id, false).await.unwrap();
        assert_eq!(closed.item.status, IssueStatus::Closed);

        let failed = store.get_issue(&failed_id, false).await.unwrap();
        assert_eq!(failed.item.status, IssueStatus::Failed);

        let dropped = store.get_issue(&dropped_id, false).await.unwrap();
        assert_eq!(dropped.item.status, IssueStatus::Dropped);
    }

    #[tokio::test]
    async fn multiple_reviews_latest_wins() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let now = Utc::now();
        let old_patch = make_patch(PatchStatus::Open, vec![]);
        let new_patch = make_patch(
            PatchStatus::Open,
            vec![
                // Earlier non-approving review
                Review::new(
                    "Changes needed".to_string(),
                    false,
                    "reviewer-a".to_string(),
                    Some(now - Duration::hours(2)),
                ),
                // Later approving review (should win)
                Review::new(
                    "LGTM now".to_string(),
                    true,
                    "reviewer-a".to_string(),
                    Some(now - Duration::hours(1)),
                ),
            ],
        );
        let (patch_id, _) = store.add_patch(new_patch.clone()).await.unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue).await.unwrap();

        let event = make_patch_updated_event(patch_id, old_patch, new_patch);

        let automation = SyncReviewRequestIssuesAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let updated = store.get_issue(&rr_id, false).await.unwrap();
        assert_eq!(
            updated.item.status,
            IssueStatus::Closed,
            "latest approving review should close the issue"
        );
    }

    #[tokio::test]
    async fn multiple_reviews_latest_non_approving_wins() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let now = Utc::now();
        let old_patch = make_patch(PatchStatus::Open, vec![]);
        let new_patch = make_patch(
            PatchStatus::Open,
            vec![
                // Earlier approving review
                Review::new(
                    "LGTM".to_string(),
                    true,
                    "reviewer-a".to_string(),
                    Some(now - Duration::hours(2)),
                ),
                // Later non-approving review (should win)
                Review::new(
                    "Actually, changes needed".to_string(),
                    false,
                    "reviewer-a".to_string(),
                    Some(now - Duration::hours(1)),
                ),
            ],
        );
        let (patch_id, _) = store.add_patch(new_patch.clone()).await.unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue).await.unwrap();

        let event = make_patch_updated_event(patch_id, old_patch, new_patch);

        let automation = SyncReviewRequestIssuesAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let updated = store.get_issue(&rr_id, false).await.unwrap();
        assert_eq!(
            updated.item.status,
            IssueStatus::Failed,
            "latest non-approving review should fail the issue"
        );
    }

    #[tokio::test]
    async fn case_insensitive_author_matching() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let old_patch = make_patch(PatchStatus::Open, vec![]);
        let new_patch = make_patch(
            PatchStatus::Open,
            vec![Review::new(
                "LGTM".to_string(),
                true,
                "Reviewer-A".to_string(), // Different case
                Some(Utc::now()),
            )],
        );
        let (patch_id, _) = store.add_patch(new_patch.clone()).await.unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue).await.unwrap();

        let event = make_patch_updated_event(patch_id, old_patch, new_patch);

        let automation = SyncReviewRequestIssuesAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let updated = store.get_issue(&rr_id, false).await.unwrap();
        assert_eq!(
            updated.item.status,
            IssueStatus::Closed,
            "case-insensitive author matching should work"
        );
    }

    #[tokio::test]
    async fn multiple_assignees_handled_independently() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let old_patch = make_patch(PatchStatus::Open, vec![]);
        let new_patch = make_patch(
            PatchStatus::Open,
            vec![
                Review::new(
                    "LGTM".to_string(),
                    true,
                    "reviewer-a".to_string(),
                    Some(Utc::now()),
                ),
                Review::new(
                    "Changes needed".to_string(),
                    false,
                    "reviewer-b".to_string(),
                    Some(Utc::now()),
                ),
            ],
        );
        let (patch_id, _) = store.add_patch(new_patch.clone()).await.unwrap();

        let rr_a = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_a_id, _) = store.add_issue(rr_a).await.unwrap();

        let rr_b = make_review_request_issue(&patch_id, "reviewer-b", IssueStatus::Open);
        let (rr_b_id, _) = store.add_issue(rr_b).await.unwrap();

        let event = make_patch_updated_event(patch_id, old_patch, new_patch);

        let automation = SyncReviewRequestIssuesAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let updated_a = store.get_issue(&rr_a_id, false).await.unwrap();
        assert_eq!(
            updated_a.item.status,
            IssueStatus::Closed,
            "reviewer-a approved, so their review-request should be closed"
        );

        let updated_b = store.get_issue(&rr_b_id, false).await.unwrap();
        assert_eq!(
            updated_b.item.status,
            IssueStatus::Failed,
            "reviewer-b did not approve, so their review-request should be failed"
        );
    }

    #[tokio::test]
    async fn no_reviews_does_nothing() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let old_patch = make_patch(PatchStatus::Open, vec![]);
        let new_patch = make_patch(PatchStatus::Open, vec![]);
        let (patch_id, _) = store.add_patch(new_patch.clone()).await.unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue).await.unwrap();

        let event = make_patch_updated_event(patch_id, old_patch, new_patch);

        let automation = SyncReviewRequestIssuesAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let updated = store.get_issue(&rr_id, false).await.unwrap();
        assert_eq!(
            updated.item.status,
            IssueStatus::Open,
            "no reviews should leave issue unchanged"
        );
    }

    #[tokio::test]
    async fn unmatched_reviewer_leaves_issue_unchanged() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let old_patch = make_patch(PatchStatus::Open, vec![]);
        let new_patch = make_patch(
            PatchStatus::Open,
            vec![Review::new(
                "LGTM".to_string(),
                true,
                "other-reviewer".to_string(),
                Some(Utc::now()),
            )],
        );
        let (patch_id, _) = store.add_patch(new_patch.clone()).await.unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue).await.unwrap();

        let event = make_patch_updated_event(patch_id, old_patch, new_patch);

        let automation = SyncReviewRequestIssuesAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let updated = store.get_issue(&rr_id, false).await.unwrap();
        assert_eq!(
            updated.item.status,
            IssueStatus::Open,
            "review from a different author should not affect the issue"
        );
    }

    #[tokio::test]
    async fn in_progress_review_request_also_synced() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let old_patch = make_patch(PatchStatus::Open, vec![]);
        let new_patch = make_patch(
            PatchStatus::Open,
            vec![Review::new(
                "LGTM".to_string(),
                true,
                "reviewer-a".to_string(),
                Some(Utc::now()),
            )],
        );
        let (patch_id, _) = store.add_patch(new_patch.clone()).await.unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::InProgress);
        let (rr_id, _) = store.add_issue(rr_issue).await.unwrap();

        let event = make_patch_updated_event(patch_id, old_patch, new_patch);

        let automation = SyncReviewRequestIssuesAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let updated = store.get_issue(&rr_id, false).await.unwrap();
        assert_eq!(
            updated.item.status,
            IssueStatus::Closed,
            "in-progress review-request should also be synced"
        );
    }

    #[test]
    fn find_latest_review_picks_most_recent() {
        let now = Utc::now();
        let reviews = vec![
            Review::new(
                "old".to_string(),
                false,
                "alice".to_string(),
                Some(now - Duration::hours(3)),
            ),
            Review::new(
                "newer".to_string(),
                true,
                "alice".to_string(),
                Some(now - Duration::hours(1)),
            ),
            Review::new("other".to_string(), true, "bob".to_string(), Some(now)),
        ];

        let result = find_latest_review_by_author(&reviews, "alice").unwrap();
        assert!(result.is_approved);
        assert_eq!(result.contents, "newer");
    }

    #[test]
    fn find_latest_review_case_insensitive() {
        let reviews = vec![Review::new(
            "ok".to_string(),
            true,
            "Alice".to_string(),
            Some(Utc::now()),
        )];

        let result = find_latest_review_by_author(&reviews, "alice");
        assert!(result.is_some());
        assert!(result.unwrap().is_approved);
    }

    #[test]
    fn find_latest_review_no_match() {
        let reviews = vec![Review::new(
            "ok".to_string(),
            true,
            "bob".to_string(),
            Some(Utc::now()),
        )];

        let result = find_latest_review_by_author(&reviews, "alice");
        assert!(result.is_none());
    }
}
