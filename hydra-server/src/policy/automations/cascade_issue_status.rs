use async_trait::async_trait;
use std::collections::{HashMap, HashSet};

use crate::app::AppState;
use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::app::projects::{ResolveStatusError, project_cached, resolve_status_with_cache};
use crate::domain::actors::ActorRef;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};
use hydra_common::api::v1::projects::{Project, StatusKey};
use hydra_common::{IssueId, ProjectId};

const AUTOMATION_NAME: &str = "cascade_issue_status";

/// When an issue transitions into a status whose `cascades_to_children`
/// flag is `true`, recursively transition every non-`unblocks_parents`
/// descendant to the **same status key** the parent landed in (per child's
/// own project resolution).
///
/// The trigger is data-driven via `resolve_status(...).cascades_to_children`,
/// not a hardcoded enum list.
pub struct CascadeIssueStatusAutomation;

impl CascadeIssueStatusAutomation {
    pub fn new(_params: Option<&serde_yaml_ng::Value>) -> Result<Self, String> {
        Ok(Self)
    }
}

#[async_trait]
impl Automation for CascadeIssueStatusAutomation {
    fn name(&self) -> &str {
        AUTOMATION_NAME
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![EventType::IssueUpdated],
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        // Skip events triggered by this automation to avoid infinite loops.
        if let ActorRef::Automation {
            automation_name, ..
        } = ctx.actor()
        {
            if automation_name == AUTOMATION_NAME {
                return Ok(());
            }
        }

        let ServerEvent::IssueUpdated {
            issue_id, payload, ..
        } = ctx.event
        else {
            return Ok(());
        };

        let MutationPayload::Issue {
            old: Some(old),
            new,
            ..
        } = payload.as_ref()
        else {
            return Ok(());
        };

        tracing::info!(
            automation = AUTOMATION_NAME,
            issue_id = %issue_id,
            "automation invoked",
        );

        if old.status == new.status {
            return Ok(());
        }
        let resolved = match ctx.app_state.resolve_status(new).await {
            Ok(def) => def,
            Err(err) => {
                tracing::warn!(
                    automation = AUTOMATION_NAME,
                    issue_id = %issue_id,
                    status = %new.status,
                    error = %err,
                    "cascade_issue_status: failed to resolve new status; skipping cascade"
                );
                return Ok(());
            }
        };
        if !resolved.cascades_to_children {
            return Ok(());
        }

        let target_key = resolved.key.clone();

        let actor = ActorRef::Automation {
            automation_name: AUTOMATION_NAME.into(),
            triggered_by: Some(Box::new(ctx.actor().clone())),
        };

        cascade_to_descendants(ctx.app_state, issue_id, &target_key, actor).await?;

        tracing::info!(
            issue_id = %issue_id,
            new_status = %new.status,
            "cascade_issue_status completed"
        );

        Ok(())
    }
}

/// Helper to update an issue via `AppState::upsert_issue`.
async fn upsert_issue(
    app_state: &AppState,
    issue_id: &IssueId,
    issue: crate::domain::issues::Issue,
    actor: ActorRef,
) -> Result<(), AutomationError> {
    app_state
        .upsert_issue(
            Some(issue_id.clone()),
            hydra_common::api::v1::issues::UpsertIssueRequest::new(issue.into(), None),
            actor,
        )
        .await
        .map_err(|e| {
            AutomationError::Other(anyhow::anyhow!("failed to update issue {issue_id}: {e}"))
        })?;
    Ok(())
}

/// Recursively transition every non-`unblocks_parents` descendant to
/// `target_key` (per child's own project resolution). Cross-project
/// children whose project has no matching key are skipped with a warning.
async fn cascade_to_descendants(
    app_state: &AppState,
    issue_id: &IssueId,
    target_key: &StatusKey,
    actor: ActorRef,
) -> Result<(), AutomationError> {
    let store = app_state.store();
    let mut to_visit = store.get_issue_children(issue_id).await.map_err(|e| {
        AutomationError::Other(anyhow::anyhow!("failed to get children of {issue_id}: {e}"))
    })?;

    let mut visited = HashSet::new();
    // Same-project descendants are the dominant case (the default project
    // covers most graphs); cache resolved projects across the traversal so
    // each project is fetched at most once for both the current-status
    // resolve and the target-key declaration check.
    let mut project_cache: HashMap<ProjectId, Project> = HashMap::new();

    while let Some(child_id) = to_visit.pop() {
        if !visited.insert(child_id.clone()) {
            continue;
        }

        let child = store.get_issue(&child_id, false).await.map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to fetch child issue {child_id}: {e}"
            ))
        })?;

        let mut child_issue = child.item;
        let resolved_current = match resolve_status_with_cache(
            &mut project_cache,
            store,
            &child_issue,
        )
        .await
        {
            Ok(def) => Some(def),
            Err(err) => {
                tracing::warn!(
                    automation = AUTOMATION_NAME,
                    child_id = %child_id,
                    status = %child_issue.status,
                    error = %err,
                    "cascade_issue_status: child has unresolvable current status; skipping but still descending"
                );
                None
            }
        };
        let is_terminal_child = resolved_current
            .as_ref()
            .is_some_and(|d| d.unblocks_parents);

        if !is_terminal_child {
            // Check that the child's resolved project declares `target_key`.
            // Cross-project children where the key is missing are skipped
            // and logged.
            let target_in_child_project =
                child_project_has_key(&mut project_cache, store, &child_issue, target_key).await;
            match target_in_child_project {
                Ok(true) => {
                    child_issue.status = target_key.clone();
                    upsert_issue(app_state, &child_id, child_issue, actor.clone()).await?;
                }
                Ok(false) => {
                    tracing::warn!(
                        automation = AUTOMATION_NAME,
                        child_id = %child_id,
                        target_status = %target_key,
                        project_id = ?child_issue.project_id,
                        "cascade_issue_status: child's project does not declare cascade target; skipping"
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        automation = AUTOMATION_NAME,
                        child_id = %child_id,
                        target_status = %target_key,
                        error = %err,
                        "cascade_issue_status: failed to look up child's project; skipping"
                    );
                }
            }
        }

        let grandchildren = store.get_issue_children(&child_id).await.map_err(|e| {
            AutomationError::Other(anyhow::anyhow!("failed to get children of {child_id}: {e}"))
        })?;
        to_visit.extend(grandchildren);
    }

    Ok(())
}

/// Returns whether the child's resolved project declares a status with
/// `target_key`. Errors propagate to the caller, which logs and skips.
/// Routed through `project_cache` so a project shared with prior children
/// is fetched only once per `cascade_to_descendants` call.
async fn child_project_has_key(
    cache: &mut HashMap<ProjectId, Project>,
    store: &dyn crate::store::ReadOnlyStore,
    child: &crate::domain::issues::Issue,
    target_key: &StatusKey,
) -> Result<bool, anyhow::Error> {
    match project_cached(cache, store, &child.project_id).await {
        Ok(project) => Ok(project.find_status(target_key).is_some()),
        Err(ResolveStatusError::ProjectNotFound(_)) => Ok(false),
        Err(err) => Err(anyhow::anyhow!(
            "store error reading project {}: {err}",
            child.project_id
        )),
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
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use crate::test_utils;
    use chrono::Utc;
    use std::sync::Arc;

    fn make_issue(status: IssueStatus, deps: Vec<IssueDependency>) -> Issue {
        Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "test".to_string(),
            Username::from("tester"),
            String::new(),
            status.into(),
            crate::domain::projects::default_project_id(),
            None,
            None,
            deps,
            Vec::new(),
            None,
            None,
            None,
        )
    }

    #[tokio::test]
    async fn drops_children_when_parent_dropped() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        // Create parent and child
        let parent = make_issue(IssueStatus::Open, Vec::new());
        let (parent_id, _) = store
            .add_issue(parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let child = make_issue(
            IssueStatus::Open,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, _) = store.add_issue(child, &ActorRef::test()).await.unwrap();

        // Update parent to Dropped
        let mut dropped_parent = parent;
        dropped_parent.status = IssueStatus::Dropped.into();
        store
            .update_issue(&parent_id, dropped_parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: dropped_parent,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: parent_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let child_result = store.get_issue(&child_id, false).await.unwrap();
        assert_eq!(
            child_result.item.status,
            IssueStatus::Dropped.as_status_key()
        );
    }

    #[tokio::test]
    async fn does_not_cascade_to_blocked_on_dependents_when_failed() {
        // Cascade only follows `child-of` edges; `blocked-on` dependents
        // never receive the cascaded status, regardless of which terminal
        // key the parent landed in.
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        // Create issue A (will fail)
        let issue_a = make_issue(IssueStatus::Open, Vec::new());
        let (id_a, _) = store
            .add_issue(issue_a.clone(), &ActorRef::test())
            .await
            .unwrap();

        // Create issue B that is blocked on A
        let issue_b = make_issue(
            IssueStatus::Open,
            vec![IssueDependency::new(
                IssueDependencyType::BlockedOn,
                id_a.clone(),
            )],
        );
        let (id_b, _) = store.add_issue(issue_b, &ActorRef::test()).await.unwrap();

        // Fail issue A — B is a blocked-on dependent (not a child-of), so
        // it should stay Open even though the cascade fires for child-of
        // descendants.
        let mut failed_a = issue_a;
        failed_a.status = IssueStatus::Failed.into();
        store
            .update_issue(&id_a, failed_a.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: failed_a,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: id_a.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        // B should remain Open (not dropped)
        let b_result = store.get_issue(&id_b, false).await.unwrap();
        assert_eq!(b_result.item.status, IssueStatus::Open.as_status_key());
    }

    #[tokio::test]
    async fn fails_children_when_parent_failed() {
        // A `failed` parent cascades children to `failed`. A cascaded child
        // then reads as "failed because parent failed" rather than "dropped."
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        // Create parent and child
        let parent = make_issue(IssueStatus::Open, Vec::new());
        let (parent_id, _) = store
            .add_issue(parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let child = make_issue(
            IssueStatus::Open,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, _) = store.add_issue(child, &ActorRef::test()).await.unwrap();

        // Fail the parent — children should now also be Failed (not Dropped).
        let mut failed_parent = parent;
        failed_parent.status = IssueStatus::Failed.into();
        store
            .update_issue(&parent_id, failed_parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: failed_parent,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: parent_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let child_result = store.get_issue(&child_id, false).await.unwrap();
        assert_eq!(
            child_result.item.status,
            IssueStatus::Failed.as_status_key()
        );
    }

    #[tokio::test]
    async fn drops_children_when_parent_dropped_v2() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        // Create parent and child
        let parent = make_issue(IssueStatus::Open, Vec::new());
        let (parent_id, _) = store
            .add_issue(parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let child = make_issue(
            IssueStatus::Open,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, _) = store.add_issue(child, &ActorRef::test()).await.unwrap();

        // Drop the parent — children should be dropped.
        let mut dropped_parent = parent;
        dropped_parent.status = IssueStatus::Dropped.into();
        store
            .update_issue(&parent_id, dropped_parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: dropped_parent,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: parent_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let child_result = store.get_issue(&child_id, false).await.unwrap();
        assert_eq!(
            child_result.item.status,
            IssueStatus::Dropped.as_status_key()
        );
    }

    #[tokio::test]
    async fn no_cascade_when_status_unchanged() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let issue = make_issue(IssueStatus::Failed, Vec::new());
        let (issue_id, _) = store
            .add_issue(issue.clone(), &ActorRef::test())
            .await
            .unwrap();

        // Event where old and new status are both Failed (no change)
        let payload = Arc::new(MutationPayload::Issue {
            old: Some(issue.clone()),
            new: issue,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id,
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        // Should return Ok without doing anything
        automation.execute(&ctx).await.unwrap();
    }

    #[tokio::test]
    async fn skips_closed_child_when_parent_dropped() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let parent = make_issue(IssueStatus::Open, Vec::new());
        let (parent_id, _) = store
            .add_issue(parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let child = make_issue(
            IssueStatus::Closed,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, _) = store.add_issue(child, &ActorRef::test()).await.unwrap();

        let mut dropped_parent = parent;
        dropped_parent.status = IssueStatus::Dropped.into();
        store
            .update_issue(&parent_id, dropped_parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: dropped_parent,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: parent_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let child_result = store.get_issue(&child_id, false).await.unwrap();
        assert_eq!(
            child_result.item.status,
            IssueStatus::Closed.as_status_key()
        );
    }

    #[tokio::test]
    async fn skips_failed_child_when_parent_dropped() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let parent = make_issue(IssueStatus::Open, Vec::new());
        let (parent_id, _) = store
            .add_issue(parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let child = make_issue(
            IssueStatus::Failed,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, _) = store.add_issue(child, &ActorRef::test()).await.unwrap();

        let mut dropped_parent = parent;
        dropped_parent.status = IssueStatus::Dropped.into();
        store
            .update_issue(&parent_id, dropped_parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: dropped_parent,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: parent_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let child_result = store.get_issue(&child_id, false).await.unwrap();
        assert_eq!(
            child_result.item.status,
            IssueStatus::Failed.as_status_key()
        );
    }

    #[tokio::test]
    async fn skips_dropped_child_when_parent_dropped() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let parent = make_issue(IssueStatus::Open, Vec::new());
        let (parent_id, _) = store
            .add_issue(parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let child = make_issue(
            IssueStatus::Dropped,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, _) = store.add_issue(child, &ActorRef::test()).await.unwrap();

        let mut dropped_parent = parent;
        dropped_parent.status = IssueStatus::Dropped.into();
        store
            .update_issue(&parent_id, dropped_parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: dropped_parent,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: parent_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let child_result = store.get_issue(&child_id, false).await.unwrap();
        assert_eq!(
            child_result.item.status,
            IssueStatus::Dropped.as_status_key()
        );
    }

    #[tokio::test]
    async fn drops_grandchild_of_terminal_child() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        // Create parent -> closed child -> open grandchild
        let parent = make_issue(IssueStatus::Open, Vec::new());
        let (parent_id, _) = store
            .add_issue(parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let child = make_issue(
            IssueStatus::Closed,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, _) = store.add_issue(child, &ActorRef::test()).await.unwrap();

        let grandchild = make_issue(
            IssueStatus::Open,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                child_id.clone(),
            )],
        );
        let (grandchild_id, _) = store
            .add_issue(grandchild, &ActorRef::test())
            .await
            .unwrap();

        let mut dropped_parent = parent;
        dropped_parent.status = IssueStatus::Dropped.into();
        store
            .update_issue(&parent_id, dropped_parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: dropped_parent,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: parent_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        // Closed child should stay closed
        let child_result = store.get_issue(&child_id, false).await.unwrap();
        assert_eq!(
            child_result.item.status,
            IssueStatus::Closed.as_status_key()
        );

        // Open grandchild should be dropped
        let grandchild_result = store.get_issue(&grandchild_id, false).await.unwrap();
        assert_eq!(
            grandchild_result.item.status,
            IssueStatus::Dropped.as_status_key()
        );
    }

    #[tokio::test]
    async fn does_not_cascade_when_parent_closed() {
        // `closed.cascades_to_children = false` in DefaultProject, so a
        // closed parent leaves its children untouched.
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let parent = make_issue(IssueStatus::Open, Vec::new());
        let (parent_id, _) = store
            .add_issue(parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let child = make_issue(
            IssueStatus::Open,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, _) = store.add_issue(child, &ActorRef::test()).await.unwrap();

        let mut closed_parent = parent;
        closed_parent.status = IssueStatus::Closed.into();
        store
            .update_issue(&parent_id, closed_parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: closed_parent,
            actor: ActorRef::test(),
        });
        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: parent_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };
        automation.execute(&ctx).await.unwrap();

        let child_result = store.get_issue(&child_id, false).await.unwrap();
        assert_eq!(child_result.item.status, IssueStatus::Open.as_status_key());
    }

    #[tokio::test]
    async fn skips_events_triggered_by_this_automation() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let parent = make_issue(IssueStatus::Open, Vec::new());
        let (parent_id, _) = store
            .add_issue(parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let child = make_issue(
            IssueStatus::Open,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, _) = store.add_issue(child, &ActorRef::test()).await.unwrap();

        let mut dropped_parent = parent;
        dropped_parent.status = IssueStatus::Dropped.into();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: dropped_parent,
            actor: ActorRef::Automation {
                automation_name: AUTOMATION_NAME.to_string(),
                triggered_by: None,
            },
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: parent_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        // Child should remain Open — the automation must not act on its own events.
        let child_result = store.get_issue(&child_id, false).await.unwrap();
        assert_eq!(child_result.item.status, IssueStatus::Open.as_status_key());
    }

    /// Exercises the per-call project cache: a parent with multiple
    /// same-project descendants at varying depths is dropped, and every
    /// non-terminal descendant should be cascaded to Dropped. The cache
    /// collapses the per-child `resolve_status` + `child_project_has_key`
    /// lookups to one project fetch underneath.
    #[tokio::test]
    async fn cascades_to_many_same_project_descendants_at_varying_depths() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        // Build a parent with three direct children, where one child also
        // has two grandchildren. All share the default project.
        let parent = make_issue(IssueStatus::Open, Vec::new());
        let (parent_id, _) = store
            .add_issue(parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let mut direct_child_ids = Vec::new();
        for _ in 0..3 {
            let child = make_issue(
                IssueStatus::Open,
                vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    parent_id.clone(),
                )],
            );
            let (child_id, _) = store.add_issue(child, &ActorRef::test()).await.unwrap();
            direct_child_ids.push(child_id);
        }

        let mut grandchild_ids = Vec::new();
        for _ in 0..2 {
            let grandchild = make_issue(
                IssueStatus::InProgress,
                vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    direct_child_ids[0].clone(),
                )],
            );
            let (grandchild_id, _) = store
                .add_issue(grandchild, &ActorRef::test())
                .await
                .unwrap();
            grandchild_ids.push(grandchild_id);
        }

        let mut dropped_parent = parent;
        dropped_parent.status = IssueStatus::Dropped.into();
        store
            .update_issue(&parent_id, dropped_parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: dropped_parent,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: parent_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        for id in direct_child_ids.iter().chain(grandchild_ids.iter()) {
            let result = store.get_issue(id, false).await.unwrap();
            assert_eq!(
                result.item.status,
                IssueStatus::Dropped.as_status_key(),
                "descendant {id} should be dropped"
            );
        }
    }
}
