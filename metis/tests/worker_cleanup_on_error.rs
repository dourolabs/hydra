use anyhow::{bail, Context, Result};
use metis_common::{
    jobs::{BundleSpec, SearchJobsQuery},
    task_status::Status,
};

mod common;

use common::{init_test_server_with_remote, job_id_for_prompt, wait_for_status};

#[tokio::test]
async fn worker_run_executes_cleanup_on_error() -> Result<()> {
    let env = init_test_server_with_remote("acme/worker-cleanup-error").await?;
    let prompt = "worker cleanup executes on error";
    let repo_arg = env.service_repo_name.to_string();
    let server_url = env.server.base_url();

    env.run_as_user(vec![format!(
        "metis jobs create --repo {} --var METIS_SERVER_URL={} --var METIS_ISSUE_ID={} {}",
        repo_arg, server_url, env.current_issue_id, prompt
    )])
    .await?;

    let job_id = job_id_for_prompt(&env.client, prompt)
        .await
        .context("expected job to be created for worker cleanup error test")?;
    wait_for_status(&env.client, &job_id, Status::Running).await?;
    let job = env
        .client
        .list_jobs(&SearchJobsQuery::default())
        .await?
        .jobs
        .into_iter()
        .find(|job| job.id == job_id)
        .context("expected job to exist after creation")?;
    match job.task.context {
        BundleSpec::ServiceRepository { .. } => {}
        other => bail!("job missing service repository context: {other:?}"),
    };

    let run_error = env
        .run_as_worker_with_failure(
            vec![
                "echo \"cleanup with error\" >> README.md".to_string(),
                "git add README.md".to_string(),
            ],
            job_id.clone(),
            true,
        )
        .await
        .expect_err("worker_run should return an error when commands are configured to fail");

    wait_for_status(&env.client, &job_id, Status::Failed)
        .await
        .with_context(|| format!("job did not transition to failed: {run_error}"))?;

    Ok(())
}
