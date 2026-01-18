use anyhow::{anyhow, bail, Context, Result};
use metis::client::MetisClient;
use metis_common::{
    jobs::SearchJobsQuery, patches::SearchPatchesQuery, task_status::Status, TaskId,
};
use metis_integration::test_helpers::init_test_server_with_remote;
use std::time::Instant;
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn worker_run_creates_patch_via_override_command() -> Result<()> {
    let env = init_test_server_with_remote("acme/worker-test").await?;
    let prompt = "worker integration patch flow";
    let repo_arg = env.service_repo_name.to_string();
    let server_url = env.server.base_url();

    env.run_as_user(vec![format!(
        "metis jobs create --repo {} --var METIS_SERVER_URL={} {}",
        repo_arg, server_url, prompt
    )])
    .await?;

    let job_id = job_id_for_prompt(&env.client, prompt)
        .await
        .context("expected job to be created for worker test")?;
    wait_for_status(&env.client, &job_id, Status::Running).await?;

    let job_id_clone = job_id.clone();
    env.run_as_worker(
        vec![
            "git checkout -b metis-worker".to_string(),
            "echo \"worker content\" >> README.md".to_string(),
            "git add README.md".to_string(),
            "git commit -m \"worker update\" ".to_string(),
            "git push origin HEAD ".to_string(),
            "metis patches create --title \"integration worker patch\" --description \"created by worker override\"".to_string(),
            "echo \"worker run finished\"".to_string(),
        ],
        job_id,
    )
    .await?;

    let patches = env
        .client
        .list_patches(&SearchPatchesQuery { q: None })
        .await?
        .patches;
    let non_backup_patch = patches
        .iter()
        .find(|patch| !patch.patch.is_automatic_backup)
        .ok_or_else(|| anyhow!("expected worker override to create a non-backup patch"))?;
    assert_eq!(
        non_backup_patch.patch.service_repo_name,
        env.service_repo_name
    );
    assert_eq!(non_backup_patch.patch.title, "integration worker patch");

    let jobs = env
        .client
        .list_jobs(&SearchJobsQuery::default())
        .await?
        .jobs;
    let status = jobs
        .iter()
        .find(|job| job.id == job_id_clone)
        .map(|job| job.status_log.current_status())
        .ok_or_else(|| anyhow!("job should still exist after worker run"))?;
    assert_eq!(status, Status::Complete);

    Ok(())
}

async fn job_id_for_prompt(client: &MetisClient, prompt: &str) -> Result<TaskId> {
    let jobs = client.list_jobs(&SearchJobsQuery::default()).await?.jobs;
    jobs.into_iter()
        .find(|job| job.task.prompt == prompt)
        .map(|job| job.id)
        .ok_or_else(|| anyhow!("job with prompt '{prompt}' not found"))
}

async fn wait_for_status(client: &MetisClient, job_id: &TaskId, expected: Status) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if Instant::now() > deadline {
            bail!("timed out waiting for job '{job_id}' to reach status {expected:?}");
        }

        let jobs = client.list_jobs(&SearchJobsQuery::default()).await?.jobs;
        if let Some(job) = jobs.iter().find(|job| &job.id == job_id) {
            if job.status_log.current_status() == expected {
                return Ok(());
            }
        }

        sleep(Duration::from_millis(50)).await;
    }
}
