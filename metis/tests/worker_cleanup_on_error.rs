use anyhow::{Context, Result};
use metis_common::task_status::Status;

mod common;

use common::{init_test_server_with_remote, job_id_for_prompt, wait_for_status};

#[tokio::test]
async fn worker_run_executes_cleanup_on_error() -> Result<()> {
    let env = init_test_server_with_remote("acme/worker-cleanup-error").await?;
    let prompt = "worker cleanup executes on error";
    let repo_arg = env.service_repo_name.to_string();
    let server_url = env.server.base_url();

    env.run_as_user(vec![format!(
        "metis jobs create --repo {} --var METIS_SERVER_URL={} {}",
        repo_arg, server_url, prompt
    )])
    .await?;

    let job_id = job_id_for_prompt(&env.client, prompt)
        .await
        .context("expected job to be created for worker cleanup error test")?;
    wait_for_status(&env.client, &job_id, Status::Running).await?;

    let run_result = env
        .run_as_worker_with_failure(
            vec![
                "echo \"cleanup with error\" >> README.md".to_string(),
                "git add README.md".to_string(),
            ],
            job_id.clone(),
            true,
        )
        .await;
    assert!(
        run_result.is_err(),
        "worker_run should return an error when commands are configured to fail"
    );

    wait_for_status(&env.client, &job_id, Status::Failed).await?;

    Ok(())
}
