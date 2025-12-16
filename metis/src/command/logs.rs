use crate::{client::MetisClientInterface, command::spawn::stream_job_logs_via_server};
use anyhow::{bail, Context, Result};
use metis_common::task_status::Status;

pub async fn run(
    client: &dyn MetisClientInterface,
    id: String,
    workflow: bool,
    watch: bool,
) -> Result<()> {
    let id = id.trim();
    if id.is_empty() {
        bail!("Job ID must not be empty.");
    }

    let job_id = if workflow {
        select_workflow_task_job_id(client, id).await?
    } else {
        id.to_string()
    };

    if workflow {
        if watch {
            println!("Streaming logs for workflow '{id}' task '{job_id}' via metis-server…");
        } else {
            println!("Fetching logs for workflow '{id}' task '{job_id}' via metis-server…");
        }
    } else if watch {
        println!("Streaming logs for job '{job_id}' via metis-server…");
    } else {
        println!("Fetching logs for job '{job_id}' via metis-server…");
    }

    stream_job_logs_via_server(client, &job_id, watch).await
}

async fn select_workflow_task_job_id(
    client: &dyn MetisClientInterface,
    workflow_id: &str,
) -> Result<String> {
    let workflow = client
        .get_workflow(workflow_id)
        .await
        .with_context(|| format!("failed to load workflow '{workflow_id}'"))?;

    if matches!(workflow.status, Status::Complete | Status::Failed) {
        if let Some(job_id) = workflow.output_task_id {
            return Ok(job_id);
        }
        bail!("workflow '{workflow_id}' is missing an output task ID");
    }

    if let Some(task) = workflow.running_tasks.first() {
        return Ok(task.metis_id.clone());
    }

    if let Some(job_id) = workflow.output_task_id {
        return Ok(job_id);
    }

    bail!("workflow '{workflow_id}' has no running tasks to stream logs from")
}
