use async_trait::async_trait;

use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::actors::ActorRef;
use crate::domain::issues::{IssueStatus, IssueType};
use crate::domain::patches::PatchStatus;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};

const AUTOMATION_NAME: &str = "close_merge_request_issues";

/// When a patch status changes to Closed/Merged/ChangesRequested, close or fail
/// all associated MergeRequest issues. Additionally, when a patch reaches a
/// terminal status (Merged/Closed), drop any open ReviewRequest issues.
///
/// - Merged → close MergeRequest issues (success), drop ReviewRequest issues
/// - Closed/ChangesRequested → fail MergeRequest issues
/// - Closed → also drop ReviewRequest issues
pub struct CloseMergeRequestIssuesAutomation;

impl CloseMergeRequestIssuesAutomation {
    pub fn new(_params: Option<&toml::Value>) -> Result<Self, String> {
        Ok(Self)
    }
}

#[async_trait]
impl Automation for CloseMergeRequestIssuesAutomation {
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

        let MutationPayload::Patch {
            old: Some(old),
            new,
            ..
        } = payload.as_ref()
        else {
            return Ok(());
        };

        // Check if the status transition should trigger closing merge request issues
        let was_active = matches!(
            old.status,
            PatchStatus::Open | PatchStatus::ChangesRequested
        );
        let now_terminal = matches!(new.status, PatchStatus::Closed | PatchStatus::Merged);
        let now_changes_requested = new.status == PatchStatus::ChangesRequested
            && old.status != PatchStatus::ChangesRequested;

        if !(now_changes_requested || was_active && now_terminal) {
            return Ok(());
        }

        let status_changed_to_merged = was_active && new.status == PatchStatus::Merged;

        let store = ctx.store;
        let issue_ids = store.get_issues_for_patch(patch_id).await.map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to get issues for patch {patch_id}: {e}"
            ))
        })?;

        let actor = ActorRef::Automation {
            automation_name: AUTOMATION_NAME.into(),
            triggered_by: Some(Box::new(ctx.actor().clone())),
        };

        let mut updated_mr_ids = Vec::new();
        let mut dropped_rr_ids = Vec::new();

        for issue_id in issue_ids {
            let issue = store.get_issue(&issue_id, false).await.map_err(|e| {
                AutomationError::Other(anyhow::anyhow!("failed to fetch issue {issue_id}: {e}"))
            })?;
            let mut issue = issue.item;

            // Skip issues already in a terminal status
            if matches!(
                issue.status,
                IssueStatus::Closed | IssueStatus::Dropped | IssueStatus::Failed
            ) {
                continue;
            }

            match issue.issue_type {
                IssueType::MergeRequest => {
                    issue.status = if status_changed_to_merged {
                        IssueStatus::Closed
                    } else {
                        IssueStatus::Failed
                    };

                    ctx.app_state
                        .upsert_issue(
                            Some(issue_id.clone()),
                            metis_common::api::v1::issues::UpsertIssueRequest::new(
                                issue.into(),
                                None,
                            ),
                            actor.clone(),
                        )
                        .await
                        .map_err(|e| {
                            AutomationError::Other(anyhow::anyhow!(
                                "failed to update merge-request issue {issue_id}: {e}"
                            ))
                        })?;

                    updated_mr_ids.push(issue_id);
                }
                IssueType::ReviewRequest if was_active && now_terminal => {
                    issue.status = IssueStatus::Dropped;

                    ctx.app_state
                        .upsert_issue(
                            Some(issue_id.clone()),
                            metis_common::api::v1::issues::UpsertIssueRequest::new(
                                issue.into(),
                                None,
                            ),
                            actor.clone(),
                        )
                        .await
                        .map_err(|e| {
                            AutomationError::Other(anyhow::anyhow!(
                                "failed to drop review-request issue {issue_id}: {e}"
                            ))
                        })?;

                    dropped_rr_ids.push(issue_id);
                }
                _ => continue,
            }
        }

        if !updated_mr_ids.is_empty() {
            let issues = updated_mr_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            let new_status = if status_changed_to_merged {
                "closed"
            } else {
                "failed"
            };
            tracing::info!(
                patch_id = %patch_id,
                issues = %issues,
                status = new_status,
                "updated merge-request issues for patch"
            );
        }

        if !dropped_rr_ids.is_empty() {
            let issues = dropped_rr_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            tracing::info!(
                patch_id = %patch_id,
                issues = %issues,
                "dropped review-request issues for patch"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::event_bus::MutationPayload;
    use crate::domain::actors::ActorRef;
    use crate::domain::issues::{
        Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType,
    };
    use crate::domain::patches::{Patch, PatchStatus};
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use crate::test_utils;
    use chrono::Utc;
    use metis_common::RepoName;
    use std::sync::Arc;

    fn make_patch(status: PatchStatus) -> Patch {
        Patch::new(
            "test patch".to_string(),
            "desc".to_string(),
            String::new(),
            status,
            false,
            None,
            Username::from("test-creator"),
            Vec::new(),
            RepoName::new("test", "repo").unwrap(),
            None,
        )
    }

    fn make_merge_request_issue(
        patch_id: &metis_common::PatchId,
        parent_id: &metis_common::IssueId,
    ) -> Issue {
        Issue::new(
            IssueType::MergeRequest,
            "merge request".to_string(),
            Username::from("tester"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
            vec![patch_id.clone()],
        )
    }

    fn make_review_request_issue(
        patch_id: &metis_common::PatchId,
        parent_id: &metis_common::IssueId,
        status: IssueStatus,
    ) -> Issue {
        Issue::new(
            IssueType::ReviewRequest,
            "review request".to_string(),
            Username::from("tester"),
            String::new(),
            status,
            Some("reviewer".to_string()),
            None,
            Vec::new(),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
            vec![patch_id.clone()],
        )
    }

    #[tokio::test]
    async fn closes_merge_request_issues_on_merge() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        // Create a patch
        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store.add_patch(patch, &ActorRef::test()).await.unwrap();

        // Create a parent issue for the merge request
        let parent = Issue::new(
            IssueType::Task,
            "parent".to_string(),
            Username::from("tester"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            vec![patch_id.clone()],
        );
        let (parent_id, _) = store.add_issue(parent, &ActorRef::test()).await.unwrap();

        // Create a merge request issue linked to the patch
        let mr_issue = make_merge_request_issue(&patch_id, &parent_id);
        let (mr_id, _) = store.add_issue(mr_issue, &ActorRef::test()).await.unwrap();

        // Simulate patch merging
        let old_patch = make_patch(PatchStatus::Open);
        let new_patch = make_patch(PatchStatus::Merged);

        let payload = Arc::new(MutationPayload::Patch {
            old: Some(old_patch),
            new: new_patch,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchUpdated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CloseMergeRequestIssuesAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let mr_result = store.get_issue(&mr_id, false).await.unwrap();
        assert_eq!(mr_result.item.status, IssueStatus::Closed);
    }

    #[tokio::test]
    async fn fails_merge_request_issues_on_close() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store.add_patch(patch, &ActorRef::test()).await.unwrap();

        let parent = Issue::new(
            IssueType::Task,
            "parent".to_string(),
            Username::from("tester"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            vec![patch_id.clone()],
        );
        let (parent_id, _) = store.add_issue(parent, &ActorRef::test()).await.unwrap();

        let mr_issue = make_merge_request_issue(&patch_id, &parent_id);
        let (mr_id, _) = store.add_issue(mr_issue, &ActorRef::test()).await.unwrap();

        let old_patch = make_patch(PatchStatus::Open);
        let new_patch = make_patch(PatchStatus::Closed);

        let payload = Arc::new(MutationPayload::Patch {
            old: Some(old_patch),
            new: new_patch,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchUpdated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CloseMergeRequestIssuesAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let mr_result = store.get_issue(&mr_id, false).await.unwrap();
        assert_eq!(mr_result.item.status, IssueStatus::Failed);
    }

    #[tokio::test]
    async fn drops_review_request_issues_on_merge() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store.add_patch(patch, &ActorRef::test()).await.unwrap();

        let parent = Issue::new(
            IssueType::Task,
            "parent".to_string(),
            Username::from("tester"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            vec![patch_id.clone()],
        );
        let (parent_id, _) = store.add_issue(parent, &ActorRef::test()).await.unwrap();

        let mr_issue = make_merge_request_issue(&patch_id, &parent_id);
        let (mr_id, _) = store.add_issue(mr_issue, &ActorRef::test()).await.unwrap();

        let rr_issue = make_review_request_issue(&patch_id, &parent_id, IssueStatus::Open);
        let (rr_id, _) = store.add_issue(rr_issue, &ActorRef::test()).await.unwrap();

        let old_patch = make_patch(PatchStatus::Open);
        let new_patch = make_patch(PatchStatus::Merged);

        let payload = Arc::new(MutationPayload::Patch {
            old: Some(old_patch),
            new: new_patch,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchUpdated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CloseMergeRequestIssuesAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let mr_result = store.get_issue(&mr_id, false).await.unwrap();
        assert_eq!(mr_result.item.status, IssueStatus::Closed);

        let rr_result = store.get_issue(&rr_id, false).await.unwrap();
        assert_eq!(rr_result.item.status, IssueStatus::Dropped);
    }

    #[tokio::test]
    async fn drops_review_request_issues_on_close() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store.add_patch(patch, &ActorRef::test()).await.unwrap();

        let parent = Issue::new(
            IssueType::Task,
            "parent".to_string(),
            Username::from("tester"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            vec![patch_id.clone()],
        );
        let (parent_id, _) = store.add_issue(parent, &ActorRef::test()).await.unwrap();

        let mr_issue = make_merge_request_issue(&patch_id, &parent_id);
        let (mr_id, _) = store.add_issue(mr_issue, &ActorRef::test()).await.unwrap();

        let rr_issue = make_review_request_issue(&patch_id, &parent_id, IssueStatus::InProgress);
        let (rr_id, _) = store.add_issue(rr_issue, &ActorRef::test()).await.unwrap();

        let old_patch = make_patch(PatchStatus::Open);
        let new_patch = make_patch(PatchStatus::Closed);

        let payload = Arc::new(MutationPayload::Patch {
            old: Some(old_patch),
            new: new_patch,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchUpdated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CloseMergeRequestIssuesAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let mr_result = store.get_issue(&mr_id, false).await.unwrap();
        assert_eq!(mr_result.item.status, IssueStatus::Failed);

        let rr_result = store.get_issue(&rr_id, false).await.unwrap();
        assert_eq!(rr_result.item.status, IssueStatus::Dropped);
    }

    #[tokio::test]
    async fn skips_terminal_review_request_issues() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store.add_patch(patch, &ActorRef::test()).await.unwrap();

        let parent = Issue::new(
            IssueType::Task,
            "parent".to_string(),
            Username::from("tester"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            vec![patch_id.clone()],
        );
        let (parent_id, _) = store.add_issue(parent, &ActorRef::test()).await.unwrap();

        // Create ReviewRequest issues already in terminal statuses
        let rr_closed = make_review_request_issue(&patch_id, &parent_id, IssueStatus::Closed);
        let (rr_closed_id, _) = store.add_issue(rr_closed, &ActorRef::test()).await.unwrap();

        let rr_dropped = make_review_request_issue(&patch_id, &parent_id, IssueStatus::Dropped);
        let (rr_dropped_id, _) = store
            .add_issue(rr_dropped, &ActorRef::test())
            .await
            .unwrap();

        let rr_failed = make_review_request_issue(&patch_id, &parent_id, IssueStatus::Failed);
        let (rr_failed_id, _) = store.add_issue(rr_failed, &ActorRef::test()).await.unwrap();

        let old_patch = make_patch(PatchStatus::Open);
        let new_patch = make_patch(PatchStatus::Merged);

        let payload = Arc::new(MutationPayload::Patch {
            old: Some(old_patch),
            new: new_patch,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchUpdated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CloseMergeRequestIssuesAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        // All should remain in their original terminal statuses
        let rr_closed_result = store.get_issue(&rr_closed_id, false).await.unwrap();
        assert_eq!(rr_closed_result.item.status, IssueStatus::Closed);

        let rr_dropped_result = store.get_issue(&rr_dropped_id, false).await.unwrap();
        assert_eq!(rr_dropped_result.item.status, IssueStatus::Dropped);

        let rr_failed_result = store.get_issue(&rr_failed_id, false).await.unwrap();
        assert_eq!(rr_failed_result.item.status, IssueStatus::Failed);
    }
}
