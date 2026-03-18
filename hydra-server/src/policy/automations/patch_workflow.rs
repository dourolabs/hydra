use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::actors::ActorRef;
use crate::domain::issues::{Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType};
use crate::domain::patches::{Patch, PatchStatus};
use crate::domain::users::Username;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};
use crate::store::ReadOnlyStore;

// Re-export config types from hydra-common for backward compatibility.
pub use hydra_common::repositories::{MergeRequestConfig, RepoWorkflowConfig, ReviewRequestConfig};

const AUTOMATION_NAME: &str = "patch_workflow";

/// Top-level configuration for the patch_workflow automation.
///
/// This provides global defaults for review and merge request creation.
/// Per-repo configuration is managed via the `patch_workflow` field on the
/// repository record in the Store (database), not via static YAML overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PatchWorkflowConfig {
    pub review_requests: Vec<ReviewRequestConfig>,
    pub merge_request: Option<MergeRequestConfig>,
}

/// Resolved workflow config for a specific patch event (after per-repo lookup).
struct ResolvedWorkflow {
    review_requests: Vec<ReviewRequestConfig>,
    merge_request: Option<MergeRequestConfig>,
}

pub struct PatchWorkflowAutomation {
    config: PatchWorkflowConfig,
}

impl PatchWorkflowAutomation {
    pub fn new(params: Option<&serde_yaml_ng::Value>) -> Result<Self, String> {
        let config = if let Some(params) = params {
            serde_yaml_ng::from_value(params.clone())
                .map_err(|e| format!("invalid patch_workflow params: {e}"))?
        } else {
            // Default: create a MergeRequest issue with no assignee (backward-compatible)
            PatchWorkflowConfig {
                review_requests: Vec::new(),
                merge_request: Some(MergeRequestConfig { assignee: None }),
            }
        };
        Ok(Self { config })
    }

    /// Resolve the effective workflow config for a given repo name.
    ///
    /// Fallback chain:
    /// 1. DB repo config (`patch_workflow` field on the repository record)
    /// 2. Static global config in YAML (top-level `review_requests` / `merge_request`)
    async fn resolve_config(
        &self,
        repo_name: &hydra_common::RepoName,
        store: &dyn ReadOnlyStore,
    ) -> ResolvedWorkflow {
        // 1. Try DB first
        match store.get_repository(repo_name, false).await {
            Ok(versioned_repo) => {
                if let Some(db_config) = versioned_repo.item.patch_workflow {
                    return ResolvedWorkflow {
                        review_requests: db_config.review_requests,
                        merge_request: db_config.merge_request,
                    };
                }
            }
            Err(e) => {
                tracing::debug!(
                    repo = %repo_name,
                    error = %e,
                    "could not fetch repository from store; falling back to static config"
                );
            }
        }

        // 2. Fall back to static global config in YAML
        ResolvedWorkflow {
            review_requests: self.config.review_requests.clone(),
            merge_request: self.config.merge_request.clone(),
        }
    }

    /// Resolve `$patch_creator` variable in an assignee string.
    /// Returns None if the variable cannot be resolved (e.g. no patch creator).
    fn resolve_assignee(
        &self,
        assignee_template: &str,
        patch_creator: Option<&Username>,
    ) -> Option<String> {
        if assignee_template == "$patch_creator" {
            patch_creator.map(|c| c.as_ref().to_string())
        } else if assignee_template.contains("$patch_creator") {
            patch_creator.map(|c| assignee_template.replace("$patch_creator", c.as_ref()))
        } else {
            Some(assignee_template.to_string())
        }
    }

    fn actor_ref(&self, ctx: &AutomationContext<'_>) -> ActorRef {
        ActorRef::Automation {
            automation_name: AUTOMATION_NAME.into(),
            triggered_by: Some(Box::new(ctx.actor().clone())),
        }
    }
}

#[async_trait]
impl Automation for PatchWorkflowAutomation {
    fn name(&self) -> &str {
        AUTOMATION_NAME
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![EventType::PatchCreated, EventType::PatchUpdated],
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        let (patch_id, new) = match ctx.event {
            ServerEvent::PatchCreated {
                patch_id, payload, ..
            } => {
                let MutationPayload::Patch { new, .. } = payload.as_ref() else {
                    return Ok(());
                };
                (patch_id, new)
            }
            ServerEvent::PatchUpdated {
                patch_id, payload, ..
            } => {
                let MutationPayload::Patch { new, .. } = payload.as_ref() else {
                    return Ok(());
                };
                (patch_id, new)
            }
            _ => return Ok(()),
        };

        self.handle_patch_event(ctx, patch_id, new).await
    }
}

impl PatchWorkflowAutomation {
    /// Shared handler for both PatchCreated and PatchUpdated events.
    ///
    /// Creates workflow issues (ReviewRequest and/or MergeRequest) when the patch
    /// is Open and non-backup and has no non-stale approved review. Deduplication
    /// of individual issue types is handled in `create_workflow_issues`.
    async fn handle_patch_event(
        &self,
        ctx: &AutomationContext<'_>,
        patch_id: &hydra_common::PatchId,
        patch: &Patch,
    ) -> Result<(), AutomationError> {
        // Only create issues for non-backup patches with Open status
        if patch.status != PatchStatus::Open || patch.is_automatic_backup {
            return Ok(());
        }

        let store = ctx.store;

        // Primary guard: check if there is a non-stale approved review.
        // If so, the patch can be merged and no new workflow issues are needed.
        let patch_versions = store.get_patch_versions(patch_id).await.map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to get patch versions for {patch_id}: {e}"
            ))
        })?;
        let staleness_cutoff =
            super::review_helpers::find_last_commit_range_change_timestamp(&patch_versions);

        if super::review_helpers::has_approved_non_dismissed_review(
            &patch.reviews,
            staleness_cutoff,
        ) {
            tracing::info!(
                patch_id = %patch_id,
                "patch has non-stale approved review; skipping workflow issue creation"
            );
            return Ok(());
        }

        // Resolve the parent issue for this patch.
        let parent_issue = self.resolve_parent_issue(ctx, patch_id, patch).await?;

        self.create_workflow_issues(ctx, patch_id, patch, parent_issue.as_ref())
            .await
    }

    /// Create the workflow issues (ReviewRequests and optionally MergeRequest) for a patch.
    ///
    /// Checks for existing open/in-progress ReviewRequest and MergeRequest issues
    /// before creating new ones to prevent duplicates. Issues in terminal states
    /// (Closed, Failed, Dropped, Rejected) do not block creation of new issues.
    async fn create_workflow_issues(
        &self,
        ctx: &AutomationContext<'_>,
        patch_id: &hydra_common::PatchId,
        patch: &Patch,
        parent_issue: Option<&ParentIssueInfo>,
    ) -> Result<(), AutomationError> {
        let store = ctx.store;

        // Check for existing open/in-progress ReviewRequest and MergeRequest issues.
        let existing_issue_ids = store.get_issues_for_patch(patch_id).await.map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to get issues for patch {patch_id}: {e}"
            ))
        })?;

        let mut has_active_review_request = false;
        let mut has_active_merge_request = false;
        let mut existing_active_rr_ids = Vec::new();

        for issue_id in &existing_issue_ids {
            let issue = store.get_issue(issue_id, false).await.map_err(|e| {
                AutomationError::Other(anyhow::anyhow!("failed to fetch issue {issue_id}: {e}"))
            })?;
            if matches!(
                issue.item.status,
                IssueStatus::Open | IssueStatus::InProgress
            ) {
                match issue.item.issue_type {
                    IssueType::ReviewRequest => {
                        has_active_review_request = true;
                        existing_active_rr_ids.push(issue_id.clone());
                    }
                    IssueType::MergeRequest => {
                        has_active_merge_request = true;
                    }
                    _ => {}
                }
            }
        }

        let workflow = self
            .resolve_config(&patch.service_repo_name, ctx.store)
            .await;

        let (creator, session_settings, parent_dependencies) = if let Some(parent) = parent_issue {
            (
                parent.issue.creator.clone(),
                Some(parent.issue.session_settings.clone()),
                vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    parent.issue_id.clone(),
                )],
            )
        } else {
            (patch.creator.clone(), None, Vec::new())
        };

        let title = issue_title(patch);
        let actor = self.actor_ref(ctx);

        // Create ReviewRequest issues, or reuse existing active ones for MR dependencies.
        let review_request_issue_ids = if has_active_review_request {
            tracing::info!(
                patch_id = %patch_id,
                "open/in-progress review-request issues already exist for patch; skipping creation"
            );
            existing_active_rr_ids
        } else {
            let mut ids = Vec::new();
            for rr_config in workflow.review_requests {
                let assignee = self.resolve_assignee(&rr_config.assignee, Some(&patch.creator));

                let description =
                    format!("Review request for patch {}: {title}", patch_id.as_ref());
                let issue = Issue::new(
                    IssueType::ReviewRequest,
                    String::new(),
                    description,
                    creator.clone(),
                    String::new(),
                    IssueStatus::Open,
                    assignee,
                    session_settings.clone(),
                    Vec::new(),
                    parent_dependencies.clone(),
                    vec![patch_id.clone()],
                );

                let (issue_id, _version) = ctx
                    .app_state
                    .upsert_issue(
                        None,
                        hydra_common::api::v1::issues::UpsertIssueRequest::new(issue.into(), None),
                        actor.clone(),
                    )
                    .await
                    .map_err(|e| {
                        AutomationError::Other(anyhow::anyhow!(
                            "failed to create review-request issue for patch {patch_id}: {e}"
                        ))
                    })?;

                tracing::info!(
                    patch_id = %patch_id,
                    issue_id = %issue_id,
                    "created review-request issue for patch"
                );

                ids.push(issue_id);
            }
            ids
        };

        // Create MergeRequest issue if configured and no active one exists.
        if has_active_merge_request {
            tracing::info!(
                patch_id = %patch_id,
                "open/in-progress merge-request issue already exists for patch; skipping creation"
            );
        } else if let Some(mr_config) = workflow.merge_request {
            let assignee = mr_config
                .assignee
                .as_ref()
                .and_then(|tmpl| self.resolve_assignee(tmpl, Some(&patch.creator)));

            let description = format!("Review patch {}: {title}", patch_id.as_ref());

            // MergeRequest is blocked-on all ReviewRequest issues
            let mut dependencies = parent_dependencies.clone();
            for rr_issue_id in &review_request_issue_ids {
                dependencies.push(IssueDependency::new(
                    IssueDependencyType::BlockedOn,
                    rr_issue_id.clone(),
                ));
            }

            let issue = Issue::new(
                IssueType::MergeRequest,
                String::new(),
                description,
                creator,
                String::new(),
                IssueStatus::Open,
                assignee,
                session_settings,
                Vec::new(),
                dependencies,
                vec![patch_id.clone()],
            );

            let (issue_id, _version) = ctx
                .app_state
                .upsert_issue(
                    None,
                    hydra_common::api::v1::issues::UpsertIssueRequest::new(issue.into(), None),
                    actor,
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
                "created merge-request issue for patch (blocked on {} review requests)",
                review_request_issue_ids.len()
            );
        }

        Ok(())
    }

    /// Resolves the parent issue for a patch by tracing its lineage.
    ///
    /// First tries: `patch.created_by` (SessionId) -> `task.spawned_from` (IssueId).
    /// Fallback: finds a non-MergeRequest, non-ReviewRequest issue that references
    /// this patch via `get_issues_for_patch`.
    async fn resolve_parent_issue(
        &self,
        ctx: &AutomationContext<'_>,
        patch_id: &hydra_common::PatchId,
        patch: &Patch,
    ) -> Result<Option<ParentIssueInfo>, AutomationError> {
        let store = ctx.store;

        // Try tracing via created_by -> task.spawned_from
        if let Some(ref task_id) = patch.created_by {
            match store.get_session(task_id, false).await {
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

        // Fallback: find a non-MergeRequest, non-ReviewRequest issue that references this patch
        let issue_ids = store.get_issues_for_patch(patch_id).await.map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to get issues for patch {patch_id}: {e}"
            ))
        })?;

        for issue_id in issue_ids {
            let issue = store.get_issue(&issue_id, false).await.map_err(|e| {
                AutomationError::Other(anyhow::anyhow!("failed to fetch issue {issue_id}: {e}"))
            })?;
            if issue.item.issue_type != IssueType::MergeRequest
                && issue.item.issue_type != IssueType::ReviewRequest
            {
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
    issue_id: hydra_common::IssueId,
    issue: Issue,
}

fn issue_title(patch: &Patch) -> String {
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
    use crate::domain::actors::ActorRef;
    use crate::domain::patches::{CommitRange, GitOid, Patch, PatchStatus, Review};
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use crate::test_utils;
    use chrono::{Duration, Utc};
    use hydra_common::RepoName;
    use std::str::FromStr;
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
            None,
            None,
            None,
        )
    }

    fn make_patch_with_creator(status: PatchStatus, creator: &str) -> Patch {
        let mut patch = make_patch(status);
        patch.creator = Username::from(creator);
        patch
    }

    fn make_backup_patch(status: PatchStatus) -> Patch {
        Patch::new(
            "backup patch".to_string(),
            "desc".to_string(),
            String::new(),
            status,
            true,
            None,
            Username::from("test-creator"),
            Vec::new(),
            RepoName::new("test", "repo").unwrap(),
            None,
            None,
            None,
            None,
        )
    }

    // ---- Backward-compatible tests (ported from create_merge_request_issue) ----

    #[tokio::test]
    async fn creates_merge_request_issue_on_reopen() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::ChangesRequested);
        let (patch_id, _) = store.add_patch(patch, &ActorRef::test()).await.unwrap();

        let parent = Issue::new(
            IssueType::Task,
            "Parent Task Title".to_string(),
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
        let (parent_id, _) = store.add_issue(parent, &ActorRef::test()).await.unwrap();

        let mr_issue = Issue::new(
            IssueType::MergeRequest,
            "Merge Request Title".to_string(),
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
        let (_mr_id, _) = store.add_issue(mr_issue, &ActorRef::test()).await.unwrap();

        let old_patch = make_patch(PatchStatus::ChangesRequested);
        let new_patch = make_patch(PatchStatus::Open);

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

        let automation = PatchWorkflowAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

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

        let (patch_id, _) = store
            .add_patch(new_patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: Some(old_patch),
            new: new_patch,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchUpdated {
            seq: 1,
            patch_id,
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = PatchWorkflowAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();
    }

    #[tokio::test]
    async fn creates_merge_request_issue_on_patch_created_with_open_status() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let parent = Issue::new(
            IssueType::Task,
            "Parent Task Title".to_string(),
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
        let (_parent_id, _) = store.add_issue(parent, &ActorRef::test()).await.unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: patch,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = PatchWorkflowAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let issues = store.get_issues_for_patch(&patch_id).await.unwrap();
        let mut open_mr_count = 0;
        for issue_id in &issues {
            let issue = store.get_issue(issue_id, false).await.unwrap();
            if issue.item.issue_type == IssueType::MergeRequest
                && issue.item.status == IssueStatus::Open
            {
                assert_eq!(issue.item.assignee, None);
                open_mr_count += 1;
            }
        }
        assert_eq!(open_mr_count, 1, "expected exactly one open MR issue");
    }

    #[tokio::test]
    async fn skips_patch_created_with_non_open_status() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::ChangesRequested);
        let (patch_id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: patch,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = PatchWorkflowAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

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
        let (patch_id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: patch,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = PatchWorkflowAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

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
    async fn patch_created_without_parent_issue_uses_patch_creator() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: patch,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = PatchWorkflowAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let issues = store.get_issues_for_patch(&patch_id).await.unwrap();
        let mut found = false;
        for issue_id in &issues {
            let issue = store.get_issue(issue_id, false).await.unwrap();
            if issue.item.issue_type == IssueType::MergeRequest
                && issue.item.status == IssueStatus::Open
            {
                assert_eq!(issue.item.creator, Username::from("test-creator"));
                found = true;
            }
        }
        assert!(found, "expected a MergeRequest issue with patch creator");
    }

    // ---- New tests for patch_workflow-specific functionality ----

    #[test]
    fn config_deserializes_from_yaml() {
        let yaml_str = r#"
review_requests:
  - assignee: "jayantk"
merge_request:
  assignee: "$patch_creator"
        "#;
        let config: PatchWorkflowConfig = serde_yaml_ng::from_str(yaml_str).unwrap();
        assert_eq!(config.review_requests.len(), 1);
        assert_eq!(config.review_requests[0].assignee, "jayantk");
        assert_eq!(
            config.merge_request.as_ref().unwrap().assignee,
            Some("$patch_creator".to_string())
        );
    }

    #[test]
    fn config_empty_deserializes_to_defaults() {
        let yaml_str = "{}";
        let config: PatchWorkflowConfig = serde_yaml_ng::from_str(yaml_str).unwrap();
        assert!(config.review_requests.is_empty());
        assert!(config.merge_request.is_none());
    }

    #[test]
    fn default_config_has_merge_request() {
        let automation = PatchWorkflowAutomation::new(None).unwrap();
        assert!(automation.config.merge_request.is_some());
        assert!(
            automation
                .config
                .merge_request
                .as_ref()
                .unwrap()
                .assignee
                .is_none()
        );
        assert!(automation.config.review_requests.is_empty());
    }

    #[tokio::test]
    async fn falls_back_to_global_config_when_repo_not_in_db() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let automation = PatchWorkflowAutomation {
            config: PatchWorkflowConfig {
                review_requests: vec![ReviewRequestConfig {
                    assignee: "global-reviewer".to_string(),
                }],
                merge_request: Some(MergeRequestConfig {
                    assignee: Some("global-merger".to_string()),
                }),
            },
        };

        let repo_name = RepoName::new("unknown", "repo").unwrap();
        let resolved = automation.resolve_config(&repo_name, store.as_ref()).await;
        assert_eq!(resolved.review_requests.len(), 1);
        assert_eq!(resolved.review_requests[0].assignee, "global-reviewer");
        assert_eq!(
            resolved.merge_request.unwrap().assignee,
            Some("global-merger".to_string())
        );
    }

    // ---- Tests for DB-first resolve_config fallback behavior ----

    #[tokio::test]
    async fn db_config_used_when_present() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        // Add a repository with patch_workflow config in the DB
        let repo_name = RepoName::new("test", "repo").unwrap();
        let repo = hydra_common::Repository::new(
            "https://example.com/repo.git".to_string(),
            None,
            None,
            Some(RepoWorkflowConfig {
                review_requests: vec![ReviewRequestConfig {
                    assignee: "db-reviewer".to_string(),
                }],
                merge_request: Some(MergeRequestConfig {
                    assignee: Some("db-merger".to_string()),
                }),
            }),
        );
        store
            .add_repository(repo_name.clone(), repo, &ActorRef::test())
            .await
            .unwrap();

        // Static config should NOT be used since DB has config
        let automation = PatchWorkflowAutomation {
            config: PatchWorkflowConfig {
                review_requests: vec![ReviewRequestConfig {
                    assignee: "global-reviewer".to_string(),
                }],
                merge_request: Some(MergeRequestConfig {
                    assignee: Some("global-merger".to_string()),
                }),
            },
        };

        let resolved = automation.resolve_config(&repo_name, store.as_ref()).await;
        assert_eq!(resolved.review_requests.len(), 1);
        assert_eq!(resolved.review_requests[0].assignee, "db-reviewer");
        assert_eq!(
            resolved.merge_request.unwrap().assignee,
            Some("db-merger".to_string())
        );
    }

    #[tokio::test]
    async fn falls_back_to_global_config_when_db_has_no_patch_workflow() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        // Add a repository WITHOUT patch_workflow config
        let repo_name = RepoName::new("test", "repo").unwrap();
        let repo = hydra_common::Repository::new(
            "https://example.com/repo.git".to_string(),
            None,
            None,
            None,
        );
        store
            .add_repository(repo_name.clone(), repo, &ActorRef::test())
            .await
            .unwrap();

        // Global config should be used as fallback when DB has no patch_workflow
        let automation = PatchWorkflowAutomation {
            config: PatchWorkflowConfig {
                review_requests: vec![ReviewRequestConfig {
                    assignee: "global-reviewer".to_string(),
                }],
                merge_request: Some(MergeRequestConfig {
                    assignee: Some("global-merger".to_string()),
                }),
            },
        };

        let resolved = automation.resolve_config(&repo_name, store.as_ref()).await;
        assert_eq!(resolved.review_requests.len(), 1);
        assert_eq!(resolved.review_requests[0].assignee, "global-reviewer");
        assert_eq!(
            resolved.merge_request.unwrap().assignee,
            Some("global-merger".to_string())
        );
    }

    #[tokio::test]
    async fn db_config_takes_precedence_over_global_config() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        // Add a repository WITH patch_workflow config in DB
        let repo_name = RepoName::new("test", "repo").unwrap();
        let repo = hydra_common::Repository::new(
            "https://example.com/repo.git".to_string(),
            None,
            None,
            Some(RepoWorkflowConfig {
                review_requests: vec![ReviewRequestConfig {
                    assignee: "db-reviewer".to_string(),
                }],
                merge_request: Some(MergeRequestConfig {
                    assignee: Some("db-merger".to_string()),
                }),
            }),
        );
        store
            .add_repository(repo_name.clone(), repo, &ActorRef::test())
            .await
            .unwrap();

        let automation = PatchWorkflowAutomation {
            config: PatchWorkflowConfig {
                review_requests: vec![ReviewRequestConfig {
                    assignee: "global-reviewer".to_string(),
                }],
                merge_request: Some(MergeRequestConfig {
                    assignee: Some("global-merger".to_string()),
                }),
            },
        };

        // DB config should take precedence over global YAML config
        let resolved = automation.resolve_config(&repo_name, store.as_ref()).await;
        assert_eq!(resolved.review_requests.len(), 1);
        assert_eq!(resolved.review_requests[0].assignee, "db-reviewer");
        assert_eq!(
            resolved.merge_request.unwrap().assignee,
            Some("db-merger".to_string())
        );
    }

    #[test]
    fn resolve_assignee_patch_creator_variable() {
        let automation = PatchWorkflowAutomation {
            config: PatchWorkflowConfig::default(),
        };

        let creator = Username::from("alice");
        let result = automation.resolve_assignee("$patch_creator", Some(&creator));
        assert_eq!(result, Some("alice".to_string()));
    }

    #[test]
    fn resolve_assignee_patch_creator_variable_with_no_creator() {
        let automation = PatchWorkflowAutomation {
            config: PatchWorkflowConfig::default(),
        };

        let result = automation.resolve_assignee("$patch_creator", None);
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_assignee_literal_string() {
        let automation = PatchWorkflowAutomation {
            config: PatchWorkflowConfig::default(),
        };

        let result = automation.resolve_assignee("jayantk", Some(&Username::from("alice")));
        assert_eq!(result, Some("jayantk".to_string()));
    }

    #[tokio::test]
    async fn creates_review_request_issues_with_config() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: patch,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        // Configure with review requests only (no merge request)
        let params: serde_yaml_ng::Value = serde_yaml_ng::from_str(
            r#"
review_requests:
  - assignee: "reviewer-a"
  - assignee: "reviewer-b"
            "#,
        )
        .unwrap();

        let automation = PatchWorkflowAutomation::new(Some(&params)).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let issues = store.get_issues_for_patch(&patch_id).await.unwrap();
        let mut rr_count = 0;
        let mut mr_count = 0;
        let mut rr_assignees: Vec<String> = Vec::new();

        for issue_id in &issues {
            let issue = store.get_issue(issue_id, false).await.unwrap();
            match issue.item.issue_type {
                IssueType::ReviewRequest => {
                    rr_count += 1;
                    if let Some(a) = &issue.item.assignee {
                        rr_assignees.push(a.clone());
                    }
                }
                IssueType::MergeRequest => {
                    mr_count += 1;
                }
                _ => {}
            }
        }

        assert_eq!(rr_count, 2, "expected two ReviewRequest issues");
        assert_eq!(mr_count, 0, "expected no MergeRequest issue");
        rr_assignees.sort();
        assert_eq!(rr_assignees, vec!["reviewer-a", "reviewer-b"]);
    }

    #[tokio::test]
    async fn creates_merge_request_blocked_on_review_requests() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: patch,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        // Configure with both review requests and merge request
        let params: serde_yaml_ng::Value = serde_yaml_ng::from_str(
            r#"
review_requests:
  - assignee: "reviewer-a"
merge_request:
  assignee: "merger"
            "#,
        )
        .unwrap();

        let automation = PatchWorkflowAutomation::new(Some(&params)).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let issues = store.get_issues_for_patch(&patch_id).await.unwrap();
        let mut rr_ids = Vec::new();
        let mut mr_issue = None;

        for issue_id in &issues {
            let issue = store.get_issue(issue_id, false).await.unwrap();
            match issue.item.issue_type {
                IssueType::ReviewRequest => {
                    rr_ids.push(issue_id.clone());
                }
                IssueType::MergeRequest => {
                    mr_issue = Some(issue.item.clone());
                }
                _ => {}
            }
        }

        assert_eq!(rr_ids.len(), 1, "expected one ReviewRequest issue");
        let mr = mr_issue.expect("expected a MergeRequest issue");
        assert_eq!(mr.assignee, Some("merger".to_string()));

        // Verify blocked-on dependencies
        let blocked_on_ids: Vec<_> = mr
            .dependencies
            .iter()
            .filter(|d| d.dependency_type == IssueDependencyType::BlockedOn)
            .map(|d| d.issue_id.clone())
            .collect();

        assert_eq!(
            blocked_on_ids, rr_ids,
            "MergeRequest should be blocked on all ReviewRequest issues"
        );
    }

    #[tokio::test]
    async fn patch_creator_variable_resolved_from_patch() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch_with_creator(PatchStatus::Open, "alice");
        let (patch_id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: patch,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let params: serde_yaml_ng::Value = serde_yaml_ng::from_str(
            r#"
merge_request:
  assignee: "$patch_creator"
            "#,
        )
        .unwrap();

        let automation = PatchWorkflowAutomation::new(Some(&params)).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let issues = store.get_issues_for_patch(&patch_id).await.unwrap();
        let mut found = false;
        for issue_id in &issues {
            let issue = store.get_issue(issue_id, false).await.unwrap();
            if issue.item.issue_type == IssueType::MergeRequest {
                assert_eq!(issue.item.assignee, Some("alice".to_string()));
                found = true;
            }
        }
        assert!(
            found,
            "expected a MergeRequest issue with resolved $patch_creator assignee"
        );
    }

    #[tokio::test]
    async fn patch_created_uses_configured_assignee() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: patch,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let params: serde_yaml_ng::Value = serde_yaml_ng::from_str(
            r#"
merge_request:
  assignee: "configured-reviewer"
            "#,
        )
        .unwrap();

        let automation = PatchWorkflowAutomation::new(Some(&params)).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

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

    // ---- Tests for non-stale review check ----

    fn make_patch_with_reviews_and_commit_range(
        status: PatchStatus,
        reviews: Vec<Review>,
        commit_range: Option<CommitRange>,
    ) -> Patch {
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
            None,
            commit_range,
            None,
        )
    }

    #[tokio::test]
    async fn patch_updated_with_new_commits_no_review_creates_issues() {
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

        // v1: initial patch with first commit range
        let patch_v1 =
            make_patch_with_reviews_and_commit_range(PatchStatus::Open, vec![], range_v1.clone());
        let (patch_id, _) = store.add_patch(patch_v1, &ActorRef::test()).await.unwrap();

        // v2: new commits pushed (commit range changed), no reviews
        let patch_v2 =
            make_patch_with_reviews_and_commit_range(PatchStatus::Open, vec![], range_v2);
        store
            .update_patch(&patch_id, patch_v2.clone(), &ActorRef::test())
            .await
            .unwrap();

        let old_patch =
            make_patch_with_reviews_and_commit_range(PatchStatus::Open, vec![], range_v1);

        let payload = Arc::new(MutationPayload::Patch {
            old: Some(old_patch),
            new: patch_v2,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchUpdated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = PatchWorkflowAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

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
        assert_eq!(
            open_mr_count, 1,
            "expected workflow issues created when no non-stale review exists"
        );
    }

    #[tokio::test]
    async fn patch_updated_with_new_commits_and_non_stale_review_skips() {
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

        // v1: initial patch
        let patch_v1 =
            make_patch_with_reviews_and_commit_range(PatchStatus::Open, vec![], range_v1);
        let (patch_id, _) = store.add_patch(patch_v1, &ActorRef::test()).await.unwrap();

        // v2: new commits + a fresh approved review (submitted AFTER the commit range change)
        let fresh_review = Review::new(
            "LGTM".to_string(),
            true,
            "reviewer-a".to_string(),
            Some(now + Duration::hours(1)),
        );
        let patch_v2 = make_patch_with_reviews_and_commit_range(
            PatchStatus::Open,
            vec![fresh_review],
            range_v2.clone(),
        );
        store
            .update_patch(&patch_id, patch_v2.clone(), &ActorRef::test())
            .await
            .unwrap();

        let old_patch =
            make_patch_with_reviews_and_commit_range(PatchStatus::Open, vec![], range_v2);

        let payload = Arc::new(MutationPayload::Patch {
            old: Some(old_patch),
            new: patch_v2,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchUpdated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = PatchWorkflowAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let issues = store.get_issues_for_patch(&patch_id).await.unwrap();
        for issue_id in &issues {
            let issue = store.get_issue(issue_id, false).await.unwrap();
            assert_ne!(
                issue.item.issue_type,
                IssueType::MergeRequest,
                "should not create workflow issues when non-stale approved review exists"
            );
        }
    }

    #[tokio::test]
    async fn patch_updated_open_to_open_creates_issues_without_review() {
        // Tests that Open→Open transitions (e.g., new commits pushed) also
        // trigger workflow issue creation when there is no approved review.
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let old_patch = make_patch(PatchStatus::Open);
        let new_patch = make_patch(PatchStatus::Open);

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

        let automation = PatchWorkflowAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

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
        assert_eq!(
            open_mr_count, 1,
            "Open→Open transition should create workflow issues when no review exists"
        );
    }

    #[tokio::test]
    async fn patch_updated_with_stale_review_creates_issues() {
        // Tests that a stale review (submitted before commit range change) does
        // not prevent workflow issue creation.
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

        // v1: initial patch
        let patch_v1 =
            make_patch_with_reviews_and_commit_range(PatchStatus::Open, vec![], range_v1);
        let (patch_id, _) = store.add_patch(patch_v1, &ActorRef::test()).await.unwrap();

        // v2: new commits + a stale review (submitted BEFORE the commit range change)
        let stale_review = Review::new(
            "LGTM".to_string(),
            true,
            "reviewer-a".to_string(),
            Some(now - Duration::hours(2)),
        );
        let patch_v2 = make_patch_with_reviews_and_commit_range(
            PatchStatus::Open,
            vec![stale_review],
            range_v2.clone(),
        );
        store
            .update_patch(&patch_id, patch_v2.clone(), &ActorRef::test())
            .await
            .unwrap();

        let old_patch =
            make_patch_with_reviews_and_commit_range(PatchStatus::Open, vec![], range_v2);

        let payload = Arc::new(MutationPayload::Patch {
            old: Some(old_patch),
            new: patch_v2,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchUpdated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = PatchWorkflowAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

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
        assert_eq!(
            open_mr_count, 1,
            "stale review should not prevent workflow issue creation"
        );
    }

    #[tokio::test]
    async fn combined_handler_works_for_patch_created() {
        // Verifies the shared handler works correctly when invoked via PatchCreated event.
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: patch,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = PatchWorkflowAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

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
        assert_eq!(
            open_mr_count, 1,
            "PatchCreated should create workflow issues via shared handler"
        );
    }

    #[tokio::test]
    async fn combined_handler_works_for_patch_updated() {
        // Verifies the shared handler works correctly when invoked via PatchUpdated event.
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let old_patch = make_patch(PatchStatus::ChangesRequested);
        let new_patch = make_patch(PatchStatus::Open);

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

        let automation = PatchWorkflowAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

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
        assert_eq!(
            open_mr_count, 1,
            "PatchUpdated should create workflow issues via shared handler"
        );
    }

    // ---- Tests for deduplication of ReviewRequest and MergeRequest issues ----

    #[tokio::test]
    async fn skips_duplicate_review_request_when_open_one_exists() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        // Pre-create an open ReviewRequest issue for this patch.
        let existing_rr = Issue::new(
            IssueType::ReviewRequest,
            "Existing Review Request Title".to_string(),
            "existing review request".to_string(),
            Username::from("tester"),
            String::new(),
            IssueStatus::Open,
            Some("reviewer-a".to_string()),
            None,
            Vec::new(),
            Vec::new(),
            vec![patch_id.clone()],
        );
        store
            .add_issue(existing_rr, &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: patch,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        // Configure with review requests and merge request.
        let params: serde_yaml_ng::Value = serde_yaml_ng::from_str(
            r#"
review_requests:
  - assignee: "reviewer-a"
  - assignee: "reviewer-b"
merge_request:
  assignee: "merger"
            "#,
        )
        .unwrap();

        let automation = PatchWorkflowAutomation::new(Some(&params)).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let issues = store.get_issues_for_patch(&patch_id).await.unwrap();
        let mut rr_count = 0;
        let mut mr_count = 0;
        for issue_id in &issues {
            let issue = store.get_issue(issue_id, false).await.unwrap();
            match issue.item.issue_type {
                IssueType::ReviewRequest => rr_count += 1,
                IssueType::MergeRequest => mr_count += 1,
                _ => {}
            }
        }

        // Should NOT create new ReviewRequests (one already exists).
        assert_eq!(
            rr_count, 1,
            "should not create duplicate ReviewRequest issues when an open one exists"
        );
        // Should still create the MergeRequest since none existed.
        assert_eq!(
            mr_count, 1,
            "should still create MergeRequest when no open one exists"
        );
    }

    #[tokio::test]
    async fn creates_merge_request_when_review_requests_exist_but_no_open_mr() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        // Pre-create an open ReviewRequest.
        let existing_rr = Issue::new(
            IssueType::ReviewRequest,
            "Existing Review Request Title".to_string(),
            "existing review request".to_string(),
            Username::from("tester"),
            String::new(),
            IssueStatus::InProgress,
            Some("reviewer-a".to_string()),
            None,
            Vec::new(),
            Vec::new(),
            vec![patch_id.clone()],
        );
        store
            .add_issue(existing_rr, &ActorRef::test())
            .await
            .unwrap();

        // Pre-create a Failed MergeRequest (terminal state, should not block).
        let failed_mr = Issue::new(
            IssueType::MergeRequest,
            "Failed Merge Request Title".to_string(),
            "old merge request".to_string(),
            Username::from("tester"),
            String::new(),
            IssueStatus::Failed,
            Some("merger".to_string()),
            None,
            Vec::new(),
            Vec::new(),
            vec![patch_id.clone()],
        );
        store.add_issue(failed_mr, &ActorRef::test()).await.unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: patch,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = PatchWorkflowAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let issues = store.get_issues_for_patch(&patch_id).await.unwrap();
        let mut open_mr_count = 0;
        let mut failed_mr_count = 0;
        let mut rr_count = 0;
        for issue_id in &issues {
            let issue = store.get_issue(issue_id, false).await.unwrap();
            match issue.item.issue_type {
                IssueType::MergeRequest if issue.item.status == IssueStatus::Open => {
                    open_mr_count += 1;
                }
                IssueType::MergeRequest if issue.item.status == IssueStatus::Failed => {
                    failed_mr_count += 1;
                }
                IssueType::ReviewRequest => rr_count += 1,
                _ => {}
            }
        }

        assert_eq!(
            open_mr_count, 1,
            "should create a new MergeRequest when previous one is in terminal state"
        );
        assert_eq!(failed_mr_count, 1, "failed MR should still exist");
        assert_eq!(
            rr_count, 1,
            "should not create new ReviewRequests when active ones exist"
        );
    }

    #[tokio::test]
    async fn terminal_state_issues_do_not_block_new_issue_creation() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let patch = make_patch(PatchStatus::Open);
        let (patch_id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        // Pre-create terminal-state ReviewRequest and MergeRequest issues.
        for status in [
            IssueStatus::Closed,
            IssueStatus::Failed,
            IssueStatus::Dropped,
            IssueStatus::Rejected,
        ] {
            let rr = Issue::new(
                IssueType::ReviewRequest,
                format!("Review Request ({status:?})"),
                format!("old review request ({status:?})"),
                Username::from("tester"),
                String::new(),
                status,
                Some("reviewer".to_string()),
                None,
                Vec::new(),
                Vec::new(),
                vec![patch_id.clone()],
            );
            store.add_issue(rr, &ActorRef::test()).await.unwrap();

            let mr = Issue::new(
                IssueType::MergeRequest,
                format!("Merge Request ({status:?})"),
                format!("old merge request ({status:?})"),
                Username::from("tester"),
                String::new(),
                status,
                Some("merger".to_string()),
                None,
                Vec::new(),
                Vec::new(),
                vec![patch_id.clone()],
            );
            store.add_issue(mr, &ActorRef::test()).await.unwrap();
        }

        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: patch,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::PatchCreated {
            seq: 1,
            patch_id: patch_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        // Configure with review requests and merge request.
        let params: serde_yaml_ng::Value = serde_yaml_ng::from_str(
            r#"
review_requests:
  - assignee: "new-reviewer"
merge_request:
  assignee: "new-merger"
            "#,
        )
        .unwrap();

        let automation = PatchWorkflowAutomation::new(Some(&params)).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let issues = store.get_issues_for_patch(&patch_id).await.unwrap();
        let mut new_open_rr_count = 0;
        let mut new_open_mr_count = 0;
        for issue_id in &issues {
            let issue = store.get_issue(issue_id, false).await.unwrap();
            if issue.item.status == IssueStatus::Open {
                match issue.item.issue_type {
                    IssueType::ReviewRequest => new_open_rr_count += 1,
                    IssueType::MergeRequest => new_open_mr_count += 1,
                    _ => {}
                }
            }
        }

        assert_eq!(
            new_open_rr_count, 1,
            "terminal-state ReviewRequests should not block new ReviewRequest creation"
        );
        assert_eq!(
            new_open_mr_count, 1,
            "terminal-state MergeRequests should not block new MergeRequest creation"
        );
    }
}
