use crate::{client::MetisClientInterface, command::spawn::stream_job_logs_via_server};
use anyhow::{anyhow, bail, Result};
use metis_common::{task_status::Status, workflows::WorkflowSummary};
use reqwest::StatusCode;

pub async fn run(client: &dyn MetisClientInterface, id: String, watch: bool) -> Result<()> {
    let id = id.trim();
    if id.is_empty() {
        bail!("Job ID must not be empty.");
    }
    let id = id.to_string();

    match client.get_workflow(&id).await {
        Ok(workflow) => stream_workflow_logs(client, workflow, watch).await,
        Err(err) if is_not_found_error(&err) => stream_job_logs(client, &id, watch).await,
        Err(err) => Err(err),
    }
}

async fn stream_job_logs(
    client: &dyn MetisClientInterface,
    job_id: &str,
    watch: bool,
) -> Result<()> {
    let action = if watch { "Streaming" } else { "Fetching" };
    println!("{action} logs for job '{job_id}' via metis-server…");
    stream_job_logs_via_server(client, job_id, watch).await
}

async fn stream_workflow_logs(
    client: &dyn MetisClientInterface,
    workflow: WorkflowSummary,
    watch: bool,
) -> Result<()> {
    let target = select_workflow_log_target(&workflow)?;
    let (job_id, task_label) = match target {
        WorkflowLogTarget::Output { task_name, job_id } => {
            (job_id, format!("output task '{task_name}'"))
        }
        WorkflowLogTarget::Running { task_name, job_id } => {
            (job_id, format!("running task '{task_name}'"))
        }
    };

    let action = if watch { "Streaming" } else { "Fetching" };
    println!(
        "{action} logs for workflow '{}' {task_label} (job '{job_id}') via metis-server…",
        workflow.id,
    );

    stream_job_logs_via_server(client, &job_id, watch).await
}

enum WorkflowLogTarget {
    Output { task_name: String, job_id: String },
    Running { task_name: String, job_id: String },
}

fn select_workflow_log_target(workflow: &WorkflowSummary) -> Result<WorkflowLogTarget> {
    if workflow.status == Status::Complete {
        let job_id = workflow.output_task_id.clone().ok_or_else(|| {
            anyhow!(
                "workflow '{}' does not provide an output task id",
                workflow.id
            )
        })?;

        return Ok(WorkflowLogTarget::Output {
            task_name: workflow.output.clone(),
            job_id,
        });
    }

    if let Some(task) = workflow.running_tasks.first() {
        return Ok(WorkflowLogTarget::Running {
            task_name: task.name.clone(),
            job_id: task.metis_id.clone(),
        });
    }

    if let Some(job_id) = workflow.output_task_id.clone() {
        return Ok(WorkflowLogTarget::Output {
            task_name: workflow.output.clone(),
            job_id,
        });
    }

    bail!(
        "workflow '{}' has no running tasks and no output task id to fetch logs from",
        workflow.id
    );
}

fn is_not_found_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .and_then(|req_err| req_err.status())
            .map(|status| status == StatusCode::NOT_FOUND)
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use metis_common::{task_status::TaskStatusLog, workflows::RunningTaskSummary};

    fn workflow_summary(
        status: Status,
        running_tasks: Vec<RunningTaskSummary>,
        output_task_id: Option<&str>,
    ) -> WorkflowSummary {
        WorkflowSummary {
            id: "wf-1".into(),
            output: "final-task".into(),
            output_task_id: output_task_id.map(ToOwned::to_owned),
            prompt: None,
            notes: None,
            status,
            status_log: TaskStatusLog {
                creation_time: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                start_time: None,
                end_time: None,
                current_status: status,
            },
            running_tasks,
        }
    }

    #[test]
    fn completed_workflow_uses_output_task() {
        let summary = workflow_summary(Status::Complete, vec![], Some("job-123"));
        let target = select_workflow_log_target(&summary).expect("target");
        match target {
            WorkflowLogTarget::Output { task_name, job_id } => {
                assert_eq!(task_name, "final-task");
                assert_eq!(job_id, "job-123");
            }
            WorkflowLogTarget::Running { .. } => panic!("expected output task"),
        }
    }

    #[test]
    fn running_workflow_prefers_running_task() {
        let summary = workflow_summary(
            Status::Running,
            vec![
                RunningTaskSummary {
                    name: "first".into(),
                    metis_id: "job-first".into(),
                },
                RunningTaskSummary {
                    name: "second".into(),
                    metis_id: "job-second".into(),
                },
            ],
            Some("job-final"),
        );

        let target = select_workflow_log_target(&summary).expect("target");
        match target {
            WorkflowLogTarget::Running { task_name, job_id } => {
                assert_eq!(task_name, "first");
                assert_eq!(job_id, "job-first");
            }
            WorkflowLogTarget::Output { .. } => panic!("expected running task"),
        }
    }

    #[test]
    fn fallback_to_output_when_not_running() {
        let summary = workflow_summary(Status::Failed, vec![], Some("job-final"));
        let target = select_workflow_log_target(&summary).expect("target");
        match target {
            WorkflowLogTarget::Output { task_name, job_id } => {
                assert_eq!(task_name, "final-task");
                assert_eq!(job_id, "job-final");
            }
            WorkflowLogTarget::Running { .. } => panic!("expected output fallback"),
        }
    }

    #[test]
    fn missing_output_id_errors() {
        let summary = workflow_summary(Status::Complete, vec![], None);
        let result = select_workflow_log_target(&summary);
        assert!(result.is_err());
    }
}
