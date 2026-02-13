use async_trait::async_trait;

use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::issues::{Issue, IssueStatus, IssueType};
use crate::domain::patches::PatchStatus;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};

/// When a patch status changes from ChangesRequested to Open, create a new
/// MergeRequest issue for the patch.
///
/// Configurable via `assignee` param (defaults to existing logic based on patch creator).
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
}

fn merge_request_issue_title(patch: &crate::domain::patches::Patch) -> String {
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
    use crate::domain::issues::{IssueDependency, IssueDependencyType};
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
        });

        let event = ServerEvent::PatchUpdated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
            actor: None,
        };

        let automation = CreateMergeRequestIssueAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
            actor: None,
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
        });

        let event = ServerEvent::PatchUpdated {
            seq: 1,
            patch_id,
            version: 2,
            timestamp: Utc::now(),
            payload,
            actor: None,
        };

        let automation = CreateMergeRequestIssueAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
            actor: None,
        };

        // Should be a no-op
        automation.execute(&ctx).await.unwrap();
    }
}
