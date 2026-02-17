use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::actors::ActorRef;
use crate::domain::issues::{IssueStatus, IssueType};
use crate::domain::patches::{Patch, Review};
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};
use metis_common::versioning::Versioned;

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

        // Determine the staleness cutoff: the timestamp of the last version
        // where the patch's commit_range changed. Reviews submitted before
        // this timestamp are stale and should be ignored.
        let patch_versions = store.get_patch_versions(patch_id).await.map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to get patch versions for {patch_id}: {e}"
            ))
        })?;
        let staleness_cutoff = find_last_commit_range_change_timestamp(&patch_versions);

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

            // Find the latest non-stale review from the assignee (case-insensitive)
            let latest_review =
                find_latest_review_by_author(&new.reviews, &assignee, staleness_cutoff);

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

/// Find the latest non-stale review by a given author (case-insensitive match).
/// When multiple reviews exist from the same author, the one with the latest
/// `submitted_at` timestamp wins. Reviews without a timestamp are treated
/// as older than any review with a timestamp.
///
/// If `staleness_cutoff` is `Some`, reviews whose `submitted_at` is before the
/// cutoff are considered stale and excluded. Reviews without a `submitted_at`
/// are also considered stale when a cutoff is present.
fn find_latest_review_by_author<'a>(
    reviews: &'a [Review],
    author: &str,
    staleness_cutoff: Option<DateTime<Utc>>,
) -> Option<&'a Review> {
    reviews
        .iter()
        .filter(|r| r.author.eq_ignore_ascii_case(author))
        .filter(|r| {
            // If there is a staleness cutoff, only keep reviews submitted at or after it.
            // Reviews without a submitted_at timestamp are considered stale when a cutoff exists.
            match staleness_cutoff {
                Some(cutoff) => r.submitted_at.is_some_and(|t| t >= cutoff),
                None => true,
            }
        })
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

/// Finds the timestamp of the last version where the patch's `commit_range` changed.
///
/// Walks the version history in order and returns the timestamp of the most
/// recent version where `commit_range` differs from the previous version.
/// Returns `None` if there is only one version or the `commit_range` never changed.
fn find_last_commit_range_change_timestamp(versions: &[Versioned<Patch>]) -> Option<DateTime<Utc>> {
    let mut last_change_timestamp = None;
    for window in versions.windows(2) {
        let prev = &window[0];
        let curr = &window[1];
        if curr.item.commit_range != prev.item.commit_range {
            last_change_timestamp = Some(curr.timestamp);
        }
    }
    last_change_timestamp
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::event_bus::MutationPayload;
    use crate::domain::actors::ActorRef;
    use crate::domain::issues::{Issue, IssueStatus, IssueType};
    use crate::domain::patches::{CommitRange, GitOid, Patch, PatchStatus, Review};
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use crate::test_utils;
    use chrono::{Duration, Utc};
    use metis_common::RepoName;
    use std::str::FromStr;
    use std::sync::Arc;

    fn make_patch(status: PatchStatus, reviews: Vec<Review>) -> Patch {
        Patch::new(
            "test patch".to_string(),
            "desc".to_string(),
            String::new(),
            status,
            false,
            None,
            Username::from("test-creator"),
            reviews,
            RepoName::new("test", "repo").unwrap(),
            None,
        )
    }

    fn make_patch_with_commit_range(
        status: PatchStatus,
        reviews: Vec<Review>,
        commit_range: Option<CommitRange>,
    ) -> Patch {
        let mut patch = make_patch(status, reviews);
        patch.commit_range = commit_range;
        patch
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
        let (patch_id, _) = store
            .add_patch(new_patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue, &ActorRef::test()).await.unwrap();

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
        let (patch_id, _) = store
            .add_patch(new_patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue, &ActorRef::test()).await.unwrap();

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
        let (patch_id, _) = store
            .add_patch(new_patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        // Create ReviewRequest issues in various terminal statuses
        let closed_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Closed);
        let (closed_id, _) = store
            .add_issue(closed_issue, &ActorRef::test())
            .await
            .unwrap();

        let failed_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Failed);
        let (failed_id, _) = store
            .add_issue(failed_issue, &ActorRef::test())
            .await
            .unwrap();

        let dropped_issue =
            make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Dropped);
        let (dropped_id, _) = store
            .add_issue(dropped_issue, &ActorRef::test())
            .await
            .unwrap();

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
        let (patch_id, _) = store
            .add_patch(new_patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue, &ActorRef::test()).await.unwrap();

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
        let (patch_id, _) = store
            .add_patch(new_patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue, &ActorRef::test()).await.unwrap();

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
        let (patch_id, _) = store
            .add_patch(new_patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue, &ActorRef::test()).await.unwrap();

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
        let (patch_id, _) = store
            .add_patch(new_patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let rr_a = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_a_id, _) = store.add_issue(rr_a, &ActorRef::test()).await.unwrap();

        let rr_b = make_review_request_issue(&patch_id, "reviewer-b", IssueStatus::Open);
        let (rr_b_id, _) = store.add_issue(rr_b, &ActorRef::test()).await.unwrap();

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
        let (patch_id, _) = store
            .add_patch(new_patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue, &ActorRef::test()).await.unwrap();

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
        let (patch_id, _) = store
            .add_patch(new_patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue, &ActorRef::test()).await.unwrap();

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
        let (patch_id, _) = store
            .add_patch(new_patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::InProgress);
        let (rr_id, _) = store.add_issue(rr_issue, &ActorRef::test()).await.unwrap();

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

    // --- Unit tests for find_latest_review_by_author ---

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

        let result = find_latest_review_by_author(&reviews, "alice", None).unwrap();
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

        let result = find_latest_review_by_author(&reviews, "alice", None);
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

        let result = find_latest_review_by_author(&reviews, "alice", None);
        assert!(result.is_none());
    }

    // --- Unit tests for find_last_commit_range_change_timestamp ---

    #[test]
    fn commit_range_change_timestamp_single_version() {
        let now = Utc::now();
        let versions = vec![Versioned::new(
            make_patch(PatchStatus::Open, vec![]),
            1,
            now,
        )];

        assert_eq!(find_last_commit_range_change_timestamp(&versions), None);
    }

    #[test]
    fn commit_range_change_timestamp_no_change() {
        let now = Utc::now();
        let range = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap(),
        ));
        let versions = vec![
            Versioned::new(
                make_patch_with_commit_range(PatchStatus::Open, vec![], range.clone()),
                1,
                now - Duration::hours(2),
            ),
            Versioned::new(
                make_patch_with_commit_range(PatchStatus::Open, vec![], range.clone()),
                2,
                now - Duration::hours(1),
            ),
        ];

        assert_eq!(find_last_commit_range_change_timestamp(&versions), None);
    }

    #[test]
    fn commit_range_change_timestamp_detects_change() {
        let now = Utc::now();
        let range_v1 = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap(),
        ));
        let range_v2 = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("cccccccccccccccccccccccccccccccccccccccc").unwrap(),
        ));
        let change_ts = now - Duration::hours(1);
        let versions = vec![
            Versioned::new(
                make_patch_with_commit_range(PatchStatus::Open, vec![], range_v1),
                1,
                now - Duration::hours(2),
            ),
            Versioned::new(
                make_patch_with_commit_range(PatchStatus::Open, vec![], range_v2),
                2,
                change_ts,
            ),
        ];

        assert_eq!(
            find_last_commit_range_change_timestamp(&versions),
            Some(change_ts)
        );
    }

    #[test]
    fn commit_range_change_timestamp_picks_last_change() {
        let now = Utc::now();
        let range_v1 = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap(),
        ));
        let range_v2 = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("cccccccccccccccccccccccccccccccccccccccc").unwrap(),
        ));
        let range_v3 = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("dddddddddddddddddddddddddddddddddddddddd").unwrap(),
        ));
        let ts_second_change = now - Duration::minutes(30);
        let versions = vec![
            Versioned::new(
                make_patch_with_commit_range(PatchStatus::Open, vec![], range_v1),
                1,
                now - Duration::hours(2),
            ),
            Versioned::new(
                make_patch_with_commit_range(PatchStatus::Open, vec![], range_v2),
                2,
                now - Duration::hours(1),
            ),
            Versioned::new(
                make_patch_with_commit_range(PatchStatus::Open, vec![], range_v3),
                3,
                ts_second_change,
            ),
        ];

        assert_eq!(
            find_last_commit_range_change_timestamp(&versions),
            Some(ts_second_change)
        );
    }

    // --- Unit tests for staleness filtering ---

    #[test]
    fn find_latest_review_filters_stale_reviews() {
        let now = Utc::now();
        let cutoff = now - Duration::hours(1);
        let reviews = vec![
            // Review before the cutoff (stale)
            Review::new(
                "old LGTM".to_string(),
                true,
                "alice".to_string(),
                Some(now - Duration::hours(2)),
            ),
            // Review after the cutoff (fresh)
            Review::new(
                "changes needed".to_string(),
                false,
                "alice".to_string(),
                Some(now),
            ),
        ];

        let result = find_latest_review_by_author(&reviews, "alice", Some(cutoff)).unwrap();
        assert!(!result.is_approved);
        assert_eq!(result.contents, "changes needed");
    }

    #[test]
    fn find_latest_review_all_stale_returns_none() {
        let now = Utc::now();
        let cutoff = now - Duration::hours(1);
        let reviews = vec![Review::new(
            "old LGTM".to_string(),
            true,
            "alice".to_string(),
            Some(now - Duration::hours(2)),
        )];

        let result = find_latest_review_by_author(&reviews, "alice", Some(cutoff));
        assert!(result.is_none());
    }

    #[test]
    fn find_latest_review_no_timestamp_considered_stale_when_cutoff_present() {
        let now = Utc::now();
        let cutoff = now - Duration::hours(1);
        let reviews = vec![Review::new(
            "LGTM".to_string(),
            true,
            "alice".to_string(),
            None, // no timestamp
        )];

        let result = find_latest_review_by_author(&reviews, "alice", Some(cutoff));
        assert!(result.is_none());
    }

    // --- Integration tests for staleness in the automation ---

    #[tokio::test]
    async fn stale_review_is_ignored() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let now = Utc::now();
        let range_v1 = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap(),
        ));
        let range_v2 = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("cccccccccccccccccccccccccccccccccccccccc").unwrap(),
        ));

        // v1: patch with initial commit range (no reviews)
        let patch_v1 = make_patch_with_commit_range(PatchStatus::Open, vec![], range_v1);
        let (patch_id, _) = store.add_patch(patch_v1, &ActorRef::test()).await.unwrap();

        // v2: commit range changes (simulating a force-push) and review is added
        // but the review was submitted BEFORE the commit range change
        let stale_review = Review::new(
            "LGTM".to_string(),
            true,
            "reviewer-a".to_string(),
            Some(now - Duration::hours(2)),
        );
        let patch_v2 =
            make_patch_with_commit_range(PatchStatus::Open, vec![stale_review], range_v2);
        store
            .update_patch(&patch_id, patch_v2.clone(), &ActorRef::test())
            .await
            .unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue, &ActorRef::test()).await.unwrap();

        let old_patch = make_patch(PatchStatus::Open, vec![]);
        let event = make_patch_updated_event(patch_id, old_patch, patch_v2);

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
            "stale review (submitted before commit range change) should be ignored"
        );
    }

    #[tokio::test]
    async fn fresh_review_after_commit_range_change_is_applied() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let now = Utc::now();
        let range_v1 = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap(),
        ));
        let range_v2 = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("cccccccccccccccccccccccccccccccccccccccc").unwrap(),
        ));

        // v1: patch with initial commit range
        let patch_v1 = make_patch_with_commit_range(PatchStatus::Open, vec![], range_v1);
        let (patch_id, _) = store.add_patch(patch_v1, &ActorRef::test()).await.unwrap();

        // v2: commit range changes and a fresh review is added
        let fresh_review = Review::new(
            "LGTM".to_string(),
            true,
            "reviewer-a".to_string(),
            Some(now + Duration::hours(1)),
        );
        let patch_v2 =
            make_patch_with_commit_range(PatchStatus::Open, vec![fresh_review], range_v2);
        store
            .update_patch(&patch_id, patch_v2.clone(), &ActorRef::test())
            .await
            .unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue, &ActorRef::test()).await.unwrap();

        let old_patch = make_patch(PatchStatus::Open, vec![]);
        let event = make_patch_updated_event(patch_id, old_patch, patch_v2);

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
            "fresh review (submitted after commit range change) should close the issue"
        );
    }

    #[tokio::test]
    async fn no_commit_range_change_treats_all_reviews_as_fresh() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let now = Utc::now();
        let range = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap(),
        ));

        // v1: patch with commit range
        let patch_v1 = make_patch_with_commit_range(PatchStatus::Open, vec![], range.clone());
        let (patch_id, _) = store.add_patch(patch_v1, &ActorRef::test()).await.unwrap();

        // v2: same commit range, reviews added
        let review = Review::new(
            "LGTM".to_string(),
            true,
            "reviewer-a".to_string(),
            Some(now - Duration::hours(5)),
        );
        let patch_v2 = make_patch_with_commit_range(PatchStatus::Open, vec![review], range);
        store
            .update_patch(&patch_id, patch_v2.clone(), &ActorRef::test())
            .await
            .unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue, &ActorRef::test()).await.unwrap();

        let old_patch = make_patch(PatchStatus::Open, vec![]);
        let event = make_patch_updated_event(patch_id, old_patch, patch_v2);

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
            "without commit range change, all reviews are treated as fresh"
        );
    }

    #[tokio::test]
    async fn review_without_timestamp_ignored_when_commit_range_changed() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let range_v1 = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap(),
        ));
        let range_v2 = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("cccccccccccccccccccccccccccccccccccccccc").unwrap(),
        ));

        let patch_v1 = make_patch_with_commit_range(PatchStatus::Open, vec![], range_v1);
        let (patch_id, _) = store.add_patch(patch_v1, &ActorRef::test()).await.unwrap();

        // Review without submitted_at
        let review = Review::new("LGTM".to_string(), true, "reviewer-a".to_string(), None);
        let patch_v2 = make_patch_with_commit_range(PatchStatus::Open, vec![review], range_v2);
        store
            .update_patch(&patch_id, patch_v2.clone(), &ActorRef::test())
            .await
            .unwrap();

        let rr_issue = make_review_request_issue(&patch_id, "reviewer-a", IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue, &ActorRef::test()).await.unwrap();

        let old_patch = make_patch(PatchStatus::Open, vec![]);
        let event = make_patch_updated_event(patch_id, old_patch, patch_v2);

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
            "review without submitted_at should be ignored when commit range has changed"
        );
    }
}
