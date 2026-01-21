use anyhow::{anyhow, Context, Result};
use metis_common::{jobs::SearchJobsQuery, patches::SearchPatchesQuery, task_status::Status};

mod common;

use common::{init_test_server_with_remote, job_id_for_prompt, wait_for_status};

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
    let _outputs: Vec<common::bash_commands::CommandOutput> = env.run_as_worker(
        vec![
            "echo \"worker content\" >> README.md".to_string(),
            "git add README.md".to_string(),
            "git commit -m \"worker changes\"".to_string(),
            "metis patches create --title \"integration worker patch\" --description \"created by worker override\" --range HEAD~1..HEAD".to_string(),
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
