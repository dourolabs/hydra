use crate::{client::MetisClientInterface, command::spawn::stream_job_logs_via_server};
use anyhow::{bail, Context, Result};
use metis_common::{
    task_status::Status,
    workflows::{TaskSummary, WorkflowSummary},
};
use reqwest::StatusCode;

pub async fn run(client: &dyn MetisClientInterface, id: String, watch: bool) -> Result<()> {
    let id = id.trim();
    if id.is_empty() {
        bail!("ID must not be empty.");
    }
    let id = id.to_string();

    let mut workflow_lookup_error = None;
    if let Some(target) = match resolve_workflow_log_target(client, &id).await {
        Ok(target) => target,
        Err(err) => {
            workflow_lookup_error = Some(err);
            None
        }
    } {
        let action = if watch { "Streaming" } else { "Fetching" };
        match target.source {
            WorkflowLogSource::Output => println!(
                "{action} logs for workflow '{}' output task '{}' (job {}) via metis-server…",
                target.workflow_id, target.task_name, target.job_id
            ),
            WorkflowLogSource::Running => println!(
                "{action} logs for workflow '{}' running task '{}' (job {}) via metis-server…",
                target.workflow_id, target.task_name, target.job_id
            ),
        }
        return stream_job_logs_via_server(client, &target.job_id, watch).await;
    }

    let action = if watch { "Streaming" } else { "Fetching" };
    println!("{action} logs for job '{id}' via metis-server…");

    match stream_job_logs_via_server(client, &id, watch).await {
        Ok(()) => Ok(()),
        Err(job_err) => {
            if let Some(workflow_err) = workflow_lookup_error {
                Err(job_err.context(format!(
                    "failed to resolve '{id}' as workflow: {workflow_err}"
                )))
            } else {
                Err(job_err)
            }
        }
    }
}

struct WorkflowLogTarget {
    workflow_id: String,
    task_name: String,
    job_id: String,
    source: WorkflowLogSource,
}

enum WorkflowLogSource {
    Output,
    Running,
}

async fn resolve_workflow_log_target(
    client: &dyn MetisClientInterface,
    workflow_id: &str,
) -> Result<Option<WorkflowLogTarget>> {
    let workflow = match client.get_workflow(workflow_id).await {
        Ok(workflow) => workflow,
        Err(err) => {
            if is_not_found_error(&err) {
                return Ok(None);
            }
            return Err(err.context(format!("failed to fetch workflow '{workflow_id}'")));
        }
    };

    let target = select_workflow_task(&workflow)?;
    Ok(Some(target))
}

fn select_workflow_task(workflow: &WorkflowSummary) -> Result<WorkflowLogTarget> {
    if workflow.status == Status::Complete {
        let output_task = workflow.tasks.get(&workflow.output).with_context(|| {
            format!(
                "workflow '{}' output task '{}' not found",
                workflow.id, workflow.output
            )
        })?;

        return Ok(workflow_log_target_from_task(
            workflow,
            &workflow.output,
            output_task,
            WorkflowLogSource::Output,
        ));
    }

    let mut running_tasks: Vec<(&String, &TaskSummary)> = workflow
        .tasks
        .iter()
        .filter(|(_, task)| task.status == Status::Running)
        .collect();
    running_tasks.sort_by(|(a, _), (b, _)| a.cmp(b));

    if let Some((task_name, task)) = running_tasks.first() {
        return Ok(workflow_log_target_from_task(
            workflow,
            task_name,
            task,
            WorkflowLogSource::Running,
        ));
    }

    bail!(
        "workflow '{}' is not complete and has no running tasks to stream logs",
        workflow.id
    );
}

fn workflow_log_target_from_task(
    workflow: &WorkflowSummary,
    task_name: &str,
    task: &TaskSummary,
    source: WorkflowLogSource,
) -> WorkflowLogTarget {
    WorkflowLogTarget {
        workflow_id: workflow.id.clone(),
        task_name: task_name.to_string(),
        job_id: task.metis_id.clone(),
        source,
    }
}

fn is_not_found_error(err: &anyhow::Error) -> bool {
    err.downcast_ref::<reqwest::Error>()
        .and_then(|err| err.status())
        .map(|status| status == StatusCode::NOT_FOUND)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MockMetisClient;
    use chrono::Utc;
    use metis_common::{
        task_status::{Status, TaskStatusLog},
        workflows::TaskSummary,
    };
    use std::collections::HashMap;

    #[tokio::test]
    async fn logs_use_output_task_when_workflow_is_complete() {
        let client = MockMetisClient::default();
        client.push_workflow_summary(WorkflowSummary {
            id: "wf-123".into(),
            output: "final".into(),
            prompt: None,
            notes: None,
            status: Status::Complete,
            tasks: HashMap::from([
                (
                    "prep".into(),
                    TaskSummary {
                        metis_id: "job-prep".into(),
                        status: Status::Complete,
                    },
                ),
                (
                    "final".into(),
                    TaskSummary {
                        metis_id: "job-final".into(),
                        status: Status::Complete,
                    },
                ),
            ]),
            status_log: status_log(Status::Complete, true),
        });
        client.push_log_lines(["workflow output logs\n"]);

        run(&client, "wf-123".into(), false).await.unwrap();

        assert_eq!(
            client.recorded_log_requests(),
            vec!["job-final".to_string()]
        );
    }

    #[tokio::test]
    async fn logs_use_running_task_when_workflow_is_in_progress() {
        let client = MockMetisClient::default();
        client.push_workflow_summary(WorkflowSummary {
            id: "wf-running".into(),
            output: "final".into(),
            prompt: None,
            notes: None,
            status: Status::Running,
            tasks: HashMap::from([
                (
                    "beta".into(),
                    TaskSummary {
                        metis_id: "job-beta".into(),
                        status: Status::Running,
                    },
                ),
                (
                    "alpha".into(),
                    TaskSummary {
                        metis_id: "job-alpha".into(),
                        status: Status::Running,
                    },
                ),
            ]),
            status_log: status_log(Status::Running, false),
        });
        client.push_log_lines(["running task logs\n"]);

        run(&client, "wf-running".into(), false).await.unwrap();

        assert_eq!(
            client.recorded_log_requests(),
            vec!["job-alpha".to_string()]
        );
    }

    #[tokio::test]
    async fn logs_fall_back_to_job_when_workflow_lookup_fails() {
        let client = MockMetisClient::default();
        client.push_log_lines(["job logs\n"]);

        run(&client, "job-xyz".into(), false).await.unwrap();

        assert_eq!(client.recorded_log_requests(), vec!["job-xyz".to_string()]);
    }

    fn status_log(status: Status, has_end_time: bool) -> TaskStatusLog {
        let now = Utc::now();
        TaskStatusLog {
            creation_time: now,
            start_time: Some(now),
            end_time: has_end_time.then_some(now),
            current_status: status,
        }
    }
}
