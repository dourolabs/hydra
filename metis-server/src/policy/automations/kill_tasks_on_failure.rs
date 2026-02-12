use async_trait::async_trait;

use crate::app::event_bus::{MutationPayload, ServerEvent};
use crate::domain::issues::IssueStatus;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};
use crate::store::Status;

/// When an issue's status changes to a terminal/failure status, kill all active
/// tasks (Created/Pending/Running) for that issue.
///
/// This automation should run after `cascade_issue_status` so that cascaded
/// child/dependent issues also get their tasks killed via their own update events.
pub struct KillTasksOnFailureAutomation;

impl KillTasksOnFailureAutomation {
    pub fn new(_params: Option<&toml::Value>) -> Result<Self, String> {
        Ok(Self)
    }
}

#[async_trait]
impl Automation for KillTasksOnFailureAutomation {
    fn name(&self) -> &str {
        "kill_tasks_on_issue_failure"
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![super::issue_updated_discriminant()],
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        let ServerEvent::IssueUpdated {
            issue_id, payload, ..
        } = ctx.event
        else {
            return Ok(());
        };

        let MutationPayload::Issue {
            old: Some(old),
            new,
        } = payload.as_ref()
        else {
            return Ok(());
        };

        // Only trigger when the status changed to a terminal/failure status
        if old.status == new.status {
            return Ok(());
        }
        if !matches!(
            new.status,
            IssueStatus::Dropped | IssueStatus::Rejected | IssueStatus::Failed
        ) {
            return Ok(());
        }

        let store = ctx.store;
        let task_ids = store.get_tasks_for_issue(issue_id).await.map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to get tasks for issue {issue_id}: {e}"
            ))
        })?;

        let mut killed = 0usize;
        for task_id in task_ids {
            let task = store.get_task(&task_id, false).await.map_err(|e| {
                AutomationError::Other(anyhow::anyhow!("failed to fetch task {task_id}: {e}"))
            })?;

            if matches!(
                task.item.status,
                Status::Created | Status::Pending | Status::Running
            ) {
                match ctx.app_state.job_engine.kill_job(&task_id).await {
                    Ok(()) => {
                        killed += 1;
                        tracing::info!(
                            issue_id = %issue_id,
                            task_id = %task_id,
                            "killed task for dropped/failed issue"
                        );
                    }
                    Err(crate::job_engine::JobEngineError::NotFound(_)) => {
                        tracing::info!(
                            issue_id = %issue_id,
                            task_id = %task_id,
                            "task already missing while killing for dropped/failed issue"
                        );
                    }
                    Err(e) => {
                        return Err(AutomationError::Other(anyhow::anyhow!(
                            "failed to kill task {task_id} for issue {issue_id}: {e}"
                        )));
                    }
                }
            }
        }

        if killed > 0 {
            tracing::info!(
                issue_id = %issue_id,
                killed,
                "kill_tasks_on_issue_failure completed"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::event_bus::MutationPayload;
    use crate::domain::issues::{Issue, IssueStatus, IssueType};
    use crate::domain::jobs::BundleSpec;
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use crate::test_utils;
    use chrono::Utc;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_issue(status: IssueStatus) -> Issue {
        Issue::new(
            IssueType::Task,
            "test".to_string(),
            Username::from("tester"),
            String::new(),
            status,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    fn make_task(issue_id: &metis_common::IssueId) -> crate::domain::jobs::Task {
        crate::domain::jobs::Task::new(
            "test task".to_string(),
            BundleSpec::None,
            Some(issue_id.clone()),
            Some("worker:latest".to_string()),
            None,
            HashMap::new(),
            None,
            None,
            None,
        )
    }

    #[tokio::test]
    async fn kills_tasks_when_issue_dropped() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let issue = make_issue(IssueStatus::Open);
        let (issue_id, _) = store.add_issue(issue).await.unwrap();

        // Add a task for the issue
        let task = make_task(&issue_id);
        let (task_id, _) = store.add_task(task, Utc::now()).await.unwrap();

        // Mark task as Running
        let mut running_task = store.get_task(&task_id, false).await.unwrap().item;
        running_task.status = Status::Running;
        store.update_task(&task_id, running_task).await.unwrap();

        // Update issue to Dropped
        let old_issue = make_issue(IssueStatus::Open);
        let new_issue = make_issue(IssueStatus::Dropped);
        store
            .update_issue(&issue_id, new_issue.clone())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old_issue),
            new: new_issue,
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: issue_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = KillTasksOnFailureAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        // MockJobEngine will succeed on kill_job
        automation.execute(&ctx).await.unwrap();
    }

    #[tokio::test]
    async fn skips_when_not_a_failure_status() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let old_issue = make_issue(IssueStatus::Open);
        let new_issue = make_issue(IssueStatus::InProgress);

        let (issue_id, _) = store.add_issue(new_issue.clone()).await.unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old_issue),
            new: new_issue,
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id,
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = KillTasksOnFailureAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();
    }
}
