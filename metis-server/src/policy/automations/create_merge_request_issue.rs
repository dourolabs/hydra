use async_trait::async_trait;

use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::issues::{Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType};
use crate::domain::patches::{Patch, PatchStatus};
use crate::domain::users::Username;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};

/// Creates MergeRequest tracking issues for patches.
///
/// Handles two scenarios:
/// 1. **PatchCreated**: When a new non-backup patch is created with status `Open`,
///    creates a MergeRequest issue automatically.
/// 2. **PatchUpdated**: When a patch transitions from `ChangesRequested` to `Open`,
///    creates a new MergeRequest issue using the most recent existing one as a template.
///
/// Configurable via `assignee` param (defaults to None for new patches, or inherits
/// from the template issue on reopen).
pub struct CreateMergeRequestIssueAutomation {
    assignee: Option<String>,
}

impl CreateMergeRequestIssueAutomation {
    pub fn new(params: Option<&toml::Value>) -> Result<Self, String> {
        let assignee = if let Some(params) = params {
            let table = params
                .as_table()
                .ok_or("create_merge_request_issue params must be a table")?;
            table
                .get("assignee")
                .and_then(|v| v.as_str())
                .map(String::from)
        } else {
            None
        };
        Ok(Self { assignee })
    }
}

#[async_trait]
impl Automation for CreateMergeRequestIssueAutomation {
    fn name(&self) -> &str {
        "create_merge_request_issue"
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![EventType::PatchCreated, EventType::PatchUpdated],
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        match ctx.event {
            ServerEvent::PatchCreated {
                patch_id, payload, ..
            } => self.handle_patch_created(ctx, patch_id, payload).await,
            ServerEvent::PatchUpdated {
                patch_id, payload, ..
            } => self.handle_patch_updated(ctx, patch_id, payload).await,
            _ => Ok(()),
        }
    }
}

impl CreateMergeRequestIssueAutomation {
    async fn handle_patch_created(
        &self,
        ctx: &AutomationContext<'_>,
        patch_id: &metis_common::PatchId,
        payload: &std::sync::Arc<MutationPayload>,
    ) -> Result<(), AutomationError> {
        let MutationPayload::Patch { new, .. } = payload.as_ref() else {
            return Ok(());
        };

        // Only create MergeRequest issues for non-backup patches with Open status
        if new.status != PatchStatus::Open || new.is_automatic_backup {
            return Ok(());
        }

        let store = ctx.store;

        // Check if there's already an open/in-progress merge request for this patch
        let existing_issue_ids = store.get_issues_for_patch(patch_id).await.map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to get issues for patch {patch_id}: {e}"
            ))
        })?;

        for issue_id in &existing_issue_ids {
            let issue = store.get_issue(issue_id, false).await.map_err(|e| {
                AutomationError::Other(anyhow::anyhow!("failed to fetch issue {issue_id}: {e}"))
            })?;
            if issue.item.issue_type == IssueType::MergeRequest
                && matches!(
                    issue.item.status,
                    IssueStatus::Open | IssueStatus::InProgress
                )
            {
                tracing::info!(
                    patch_id = %patch_id,
                    issue_id = %issue_id,
                    "merge-request issue already open for patch; skipping"
                );
                return Ok(());
            }
        }

        // Resolve the parent issue for this patch.
        // Try: patch.created_by (TaskId) -> task.spawned_from (IssueId)
        // Fallback: find a non-MergeRequest issue that references this patch.
        let parent_issue = self.resolve_parent_issue(ctx, patch_id, new).await?;

        let title = merge_request_issue_title(new);
        let description = format!("Review patch {}: {title}", patch_id.as_ref());

        let assignee = self.assignee.clone();

        let (creator, job_settings, dependencies) = if let Some(ref parent) = parent_issue {
            (
                parent.issue.creator.clone(),
                Some(parent.issue.job_settings.clone()),
                vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    parent.issue_id.clone(),
                )],
            )
        } else {
            (Username::from("system"), None, Vec::new())
        };

        let issue = Issue::new(
            IssueType::MergeRequest,
            description,
            creator,
            String::new(),
            IssueStatus::Open,
            assignee,
            job_settings,
            Vec::new(),
            dependencies,
            vec![patch_id.clone()],
        );

        let (issue_id, _version) = ctx
            .app_state
            .upsert_issue(
                None,
                metis_common::api::v1::issues::UpsertIssueRequest::new(issue.into(), None),
                None,
            )
            .await
            .map_err(|e| {
                AutomationError::Other(anyhow::anyhow!(
                    "failed to create merge-request issue for patch {patch_id}: {e}"
                ))
            })?;

        tracing::info!(
            patch_id = %patch_id,
            issue_id = %issue_id,
            "created merge-request issue for new patch"
        );

        Ok(())
    }

    async fn handle_patch_updated(
        &self,
        ctx: &AutomationContext<'_>,
        patch_id: &metis_common::PatchId,
        payload: &std::sync::Arc<MutationPayload>,
    ) -> Result<(), AutomationError> {
        let MutationPayload::Patch {
            old: Some(old),
            new,
            ..
        } = payload.as_ref()
        else {
            return Ok(());
        };

        // Only trigger when status changes from ChangesRequested to Open
        if old.status != PatchStatus::ChangesRequested || new.status != PatchStatus::Open {
            return Ok(());
        }

        let store = ctx.store;

        // Find existing merge request issues for this patch
        let issue_ids = store.get_issues_for_patch(patch_id).await.map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to get issues for patch {patch_id}: {e}"
            ))
        })?;

        let mut merge_request_issues = Vec::new();
        for issue_id in issue_ids {
            let issue = store.get_issue(&issue_id, false).await.map_err(|e| {
                AutomationError::Other(anyhow::anyhow!("failed to fetch issue {issue_id}: {e}"))
            })?;
            let issue_timestamp = issue.timestamp;
            let issue = issue.item;

            if issue.issue_type != IssueType::MergeRequest {
                continue;
            }

            merge_request_issues.push((issue_timestamp, issue));
        }

        if merge_request_issues.is_empty() {
            tracing::warn!(
                patch_id = %patch_id,
                "no merge-request issues found for patch update; skipping"
            );
            return Ok(());
        }

        // Skip if there's already an open/in-progress merge request
        if merge_request_issues
            .iter()
            .any(|(_, issue)| matches!(issue.status, IssueStatus::Open | IssueStatus::InProgress))
        {
            tracing::info!(
                patch_id = %patch_id,
                "merge-request issue already open for patch; skipping"
            );
            return Ok(());
        }

        // Use the most recent merge request as a template
        merge_request_issues.sort_by_key(|(timestamp, _)| *timestamp);
        let (_, template_issue) = merge_request_issues
            .pop()
            .expect("merge_request_issues is non-empty");

        let title = merge_request_issue_title(new);
        let description = format!("Review patch {}: {title}", patch_id.as_ref());

        let assignee = self.assignee.clone().or(template_issue.assignee.clone());

        let issue = Issue::new(
            IssueType::MergeRequest,
            description,
            template_issue.creator,
            String::new(),
            IssueStatus::Open,
            assignee,
            Some(template_issue.job_settings),
            Vec::new(),
            template_issue.dependencies,
            vec![patch_id.clone()],
        );

        let (issue_id, _version) = ctx
            .app_state
            .upsert_issue(
                None,
                metis_common::api::v1::issues::UpsertIssueRequest::new(issue.into(), None),
                None,
            )
            .await
            .map_err(|e| {
                AutomationError::Other(anyhow::anyhow!(
                    "failed to create merge-request issue for patch {patch_id}: {e}"
                ))
            })?;

        tracing::info!(
            patch_id = %patch_id,
            issue_id = %issue_id,
            "created merge-request issue for patch update"
        );

        Ok(())
    }

    /// Resolves the parent issue for a patch by tracing its lineage.
    ///
    /// First tries: `patch.created_by` (TaskId) -> `task.spawned_from` (IssueId).
    /// Fallback: finds a non-MergeRequest issue that references this patch via `get_issues_for_patch`.
    async fn resolve_parent_issue(
        &self,
        ctx: &AutomationContext<'_>,
        patch_id: &metis_common::PatchId,
        patch: &Patch,
    ) -> Result<Option<ParentIssueInfo>, AutomationError> {
        let store = ctx.store;

        // Try tracing via created_by -> task.spawned_from
        if let Some(ref task_id) = patch.created_by {
            match store.get_task(task_id, false).await {
                Ok(task) => {
                    if let Some(ref issue_id) = task.item.spawned_from {
                        match store.get_issue(issue_id, false).await {
                            Ok(issue) => {
                                return Ok(Some(ParentIssueInfo {
                                    issue_id: issue_id.clone(),
                                    issue: issue.item,
                                }));
                            }
                            Err(e) => {
                                tracing::warn!(
                                    patch_id = %patch_id,
                                    issue_id = %issue_id,
                                    error = %e,
                                    "failed to fetch parent issue from task.spawned_from"
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        patch_id = %patch_id,
                        task_id = %task_id,
                        error = %e,
                        "failed to fetch task for patch.created_by"
                    );
                }
            }
        }

        // Fallback: find a non-MergeRequest issue that references this patch
        let issue_ids = store.get_issues_for_patch(patch_id).await.map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to get issues for patch {patch_id}: {e}"
            ))
        })?;

        for issue_id in issue_ids {
            let issue = store.get_issue(&issue_id, false).await.map_err(|e| {
                AutomationError::Other(anyhow::anyhow!("failed to fetch issue {issue_id}: {e}"))
            })?;
            if issue.item.issue_type != IssueType::MergeRequest {
                return Ok(Some(ParentIssueInfo {
                    issue_id,
                    issue: issue.item,
                }));
            }
        }

        Ok(None)
    }
}

struct ParentIssueInfo {
    issue_id: metis_common::IssueId,
    issue: Issue,
}

fn merge_request_issue_title(patch: &Patch) -> String {
    let summary = patch.title.trim();
    if !summary.is_empty() {
        return summary.to_string();
    }

    patch
        .description
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .unwrap_or("Patch review")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::event_bus::MutationPayload;
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
            Vec::new(),
            RepoName::new("test", "repo").unwrap(),
            None,
        )
    }

    fn make_backup_patch(status: PatchStatus) -> Patch {
        Patch::new(
            "backup patch".to_string(),
            "desc".to_string(),
            String::new(),
            status,
            true,
            None,
            Vec::new(),
            RepoName::new("test", "repo").unwrap(),
            None,
        )
    }

    #[tokio::test]
    async fn creates_merge_request_issue_on_reopen() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        // Create a patch
        let patch = make_patch(PatchStatus::ChangesRequested);
        let (patch_id, _) = store.add_patch(patch).await.unwrap();

        // Create a parent issue
        let parent = Issue::new(
            IssueType::Task,
            "parent".to_string(),
            Username::from("tester"),
            String::new(),
            IssueStatus::Open,
            Some("reviewer".to_string()),
            None,
            Vec::new(),
            Vec::new(),
            vec![patch_id.clone()],
        );
        let (parent_id, _) = store.add_issue(parent).await.unwrap();

        // Create an existing (failed) merge request issue
        let mr_issue = Issue::new(
            IssueType::MergeRequest,
            "old review".to_string(),
            Username::from("tester"),
            String::new(),
            IssueStatus::Failed,
            Some("reviewer".to_string()),
            None,
            Vec::new(),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
            vec![patch_id.clone()],
        );
        let (_mr_id, _) = store.add_issue(mr_issue).await.unwrap();

        // Simulate reopening the patch
        let old_patch = make_patch(PatchStatus::ChangesRequested);
        let new_patch = make_patch(PatchStatus::Open);

        let payload = Arc::new(MutationPayload::Patch {
            old: Some(old_patch),
            new: new_patch,
            actor: None,
        });

        let event = ServerEvent::PatchUpdated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CreateMergeRequestIssueAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        // Verify a new merge request issue was created
        let issues = store.get_issues_for_patch(&patch_id).await.unwrap();

        let mut open_mr_count = 0;
        for issue_id in &issues {
            let issue = store.get_issue(issue_id, false).await.unwrap();
            if issue.item.issue_type == IssueType::MergeRequest
                && issue.item.status == IssueStatus::Open
            {
                open_mr_count += 1;
            }
        }
        assert_eq!(open_mr_count, 1, "expected exactly one open MR issue");
    }

    #[tokio::test]
    async fn skips_when_not_changes_requested_to_open() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let old_patch = make_patch(PatchStatus::Open);
        let new_patch = make_patch(PatchStatus::Merged);

        let (patch_id, _) = store.add_patch(new_patch.clone()).await.unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: Some(old_patch),
            new: new_patch,
            actor: None,
        });

        let event = ServerEvent::PatchUpdated {
            seq: 1,
            patch_id,
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CreateMergeRequestIssueAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        // Should be a no-op
        automation.execute(&ctx).await.unwrap();
    }

    #[tokio::test]
    async fn creates_merge_request_issue_on_patch_created_with_open_status() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store.add_patch(patch.clone()).await.unwrap();

        // Create a parent issue that references this patch
        let parent = Issue::new(
            IssueType::Task,
            "parent task".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::InProgress,
            None,
            None,
            Vec::new(),
            Vec::new(),
            vec![patch_id.clone()],
        );
        let (_parent_id, _) = store.add_issue(parent).await.unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: patch,
            actor: None,
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CreateMergeRequestIssueAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        // Verify a MergeRequest issue was created
        let issues = store.get_issues_for_patch(&patch_id).await.unwrap();

        let mut open_mr_count = 0;
        for issue_id in &issues {
            let issue = store.get_issue(issue_id, false).await.unwrap();
            if issue.item.issue_type == IssueType::MergeRequest
                && issue.item.status == IssueStatus::Open
            {
                open_mr_count += 1;
                // Verify assignee is None (unassigned)
                assert_eq!(issue.item.assignee, None);
            }
        }
        assert_eq!(open_mr_count, 1, "expected exactly one open MR issue");
    }

    #[tokio::test]
    async fn skips_patch_created_with_non_open_status() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::ChangesRequested);
        let (patch_id, _) = store.add_patch(patch.clone()).await.unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: patch,
            actor: None,
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CreateMergeRequestIssueAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        // Should be a no-op
        automation.execute(&ctx).await.unwrap();

        // Verify no MergeRequest issue was created
        let issues = store.get_issues_for_patch(&patch_id).await.unwrap();
        for issue_id in &issues {
            let issue = store.get_issue(issue_id, false).await.unwrap();
            assert_ne!(
                issue.item.issue_type,
                IssueType::MergeRequest,
                "should not create MR issue for non-Open patch"
            );
        }
    }

    #[tokio::test]
    async fn skips_patch_created_for_automatic_backup() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_backup_patch(PatchStatus::Open);
        let (patch_id, _) = store.add_patch(patch.clone()).await.unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: patch,
            actor: None,
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CreateMergeRequestIssueAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        // Should be a no-op
        automation.execute(&ctx).await.unwrap();

        // Verify no MergeRequest issue was created
        let issues = store.get_issues_for_patch(&patch_id).await.unwrap();
        for issue_id in &issues {
            let issue = store.get_issue(issue_id, false).await.unwrap();
            assert_ne!(
                issue.item.issue_type,
                IssueType::MergeRequest,
                "should not create MR issue for automatic backup"
            );
        }
    }

    #[tokio::test]
    async fn patch_created_uses_configured_assignee() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store.add_patch(patch.clone()).await.unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: patch,
            actor: None,
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        // Configure the automation with an assignee
        let params = toml::Value::Table({
            let mut t = toml::map::Map::new();
            t.insert(
                "assignee".to_string(),
                toml::Value::String("configured-reviewer".to_string()),
            );
            t
        });
        let automation = CreateMergeRequestIssueAutomation::new(Some(&params)).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        // Verify the MergeRequest issue was created with the configured assignee
        let issues = store.get_issues_for_patch(&patch_id).await.unwrap();

        let mut found = false;
        for issue_id in &issues {
            let issue = store.get_issue(issue_id, false).await.unwrap();
            if issue.item.issue_type == IssueType::MergeRequest
                && issue.item.status == IssueStatus::Open
            {
                assert_eq!(issue.item.assignee, Some("configured-reviewer".to_string()));
                found = true;
            }
        }
        assert!(
            found,
            "expected a MergeRequest issue with configured assignee"
        );
    }

    #[tokio::test]
    async fn patch_created_without_parent_issue_uses_system_creator() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store.add_patch(patch.clone()).await.unwrap();

        // No parent issue references this patch

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: patch,
            actor: None,
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CreateMergeRequestIssueAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        // Verify the MergeRequest issue was created with "system" creator
        let issues = store.get_issues_for_patch(&patch_id).await.unwrap();

        let mut found = false;
        for issue_id in &issues {
            let issue = store.get_issue(issue_id, false).await.unwrap();
            if issue.item.issue_type == IssueType::MergeRequest
                && issue.item.status == IssueStatus::Open
            {
                assert_eq!(issue.item.creator, Username::from("system"));
                found = true;
            }
        }
        assert!(found, "expected a MergeRequest issue with system creator");
    }
}
