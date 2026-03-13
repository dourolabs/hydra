use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use metis::command::output::{CommandContext, ResolvedOutputFormat};
use metis::worker_commands::WorkerCommands;
use metis_common::{
    constants::{ENV_METIS_ISSUE_ID, ENV_METIS_SERVER_URL, ENV_METIS_TOKEN},
    patches::SearchPatchesQuery,
    sessions::SearchSessionsQuery,
    task_status::Status,
    PatchId, SessionId,
};

use metis_server::domain::actors::ActorRef;

use super::TestHarness;

/// Output captured from a single command executed by the worker.
#[derive(Clone, Debug)]
pub struct CommandOutput {
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub status: i32,
}

/// Structured result from a successful worker run.
pub struct WorkerResult {
    /// Outputs from each command executed by the worker.
    pub outputs: Vec<CommandOutput>,
    /// Patches created during the worker run (excludes automatic backups).
    pub patches_created: Vec<PatchId>,
    /// The job's final status after the worker run.
    pub final_status: Status,
}

/// Structured result from a worker run that was expected to fail.
pub struct WorkerFailure {
    /// The error returned by the worker run.
    pub error: anyhow::Error,
    /// Outputs from commands executed before the failure.
    pub outputs: Vec<CommandOutput>,
    /// The job's final status after the failed worker run.
    pub final_status: Status,
}

/// Internal `WorkerCommands` implementation that executes shell commands,
/// replacing `metis` invocations with the test binary.
struct BashCommands {
    commands: Vec<String>,
    outputs: Arc<Mutex<Vec<CommandOutput>>>,
    fail_after_run: bool,
}

impl BashCommands {
    fn new(commands: Vec<String>, fail_after_run: bool) -> Self {
        Self {
            commands,
            outputs: Arc::new(Mutex::new(Vec::new())),
            fail_after_run,
        }
    }

    fn outputs(&self) -> Vec<CommandOutput> {
        self.outputs
            .lock()
            .expect("failed to lock command outputs")
            .clone()
    }

    async fn run_custom_command(
        &self,
        command_string: &str,
        working_dir: &Path,
        env: &HashMap<String, String>,
    ) -> Result<CommandOutput> {
        let first_token = command_string.split_whitespace().next();
        let command_to_run = if first_token == Some("metis") {
            let metis_path = metis_bin();
            command_string.replacen("metis", &metis_path.to_string_lossy(), 1)
        } else {
            command_string.to_string()
        };

        let output = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(&command_to_run)
            .current_dir(working_dir)
            .envs(env)
            .output()
            .await
            .context("failed to spawn custom run command")?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let status_code = output.status.code().unwrap_or(-1);
        let command_output = CommandOutput {
            command: command_to_run.clone(),
            stdout: stdout.clone(),
            stderr: stderr.clone(),
            status: status_code,
        };
        self.outputs
            .lock()
            .expect("failed to store command outputs")
            .push(command_output.clone());

        if !output.status.success() {
            bail!(
                "custom run command '{command_to_run}' failed with status {status}\nstdout:\n{stdout}\nstderr:\n{stderr}",
                status = output.status,
            );
        }

        Ok(command_output)
    }
}

#[async_trait]
impl WorkerCommands for BashCommands {
    async fn run(
        &self,
        _prompt: &str,
        _model: Option<&str>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        _output_path: &Path,
    ) -> Result<String> {
        let mut last_output = String::new();
        for command_string in &self.commands {
            let output = self
                .run_custom_command(command_string, working_dir, env)
                .await
                .with_context(|| format!("failed to run command '{command_string}'"))?;
            last_output = output.stdout.clone();
        }

        if self.fail_after_run {
            bail!("BashCommands configured to fail after running commands");
        }

        Ok(last_output)
    }
}

fn metis_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_metis"))
}

/// Ensure the job's environment variables include the server URL, auth
/// token, and issue ID so that subprocess commands (e.g. `metis patches
/// create`) can reach the test server and resolve the current issue.
/// Updates the task record in the store directly.
async fn ensure_worker_env_vars(harness: &TestHarness, job_id: &SessionId) -> Result<()> {
    let store = harness.store();
    let versioned_task = store
        .get_session(job_id, false)
        .await
        .context("failed to get task to check env vars")?;

    let mut task = versioned_task.item;
    let mut changed = false;

    if !task.env_vars.contains_key(ENV_METIS_SERVER_URL) {
        task.env_vars
            .insert(ENV_METIS_SERVER_URL.to_string(), harness.server_url());
        changed = true;
    }
    if !task.env_vars.contains_key(ENV_METIS_TOKEN) {
        task.env_vars.insert(
            ENV_METIS_TOKEN.to_string(),
            harness.default_user_token().to_string(),
        );
        changed = true;
    }
    if !task.env_vars.contains_key(ENV_METIS_ISSUE_ID) {
        if let Some(issue_id) = &task.spawned_from {
            task.env_vars
                .insert(ENV_METIS_ISSUE_ID.to_string(), issue_id.to_string());
            changed = true;
        }
    }

    if changed {
        store
            .update_session(job_id, task, &ActorRef::test())
            .await
            .context("failed to update task env vars for worker")?;
    }

    Ok(())
}

/// Collect patches created during the worker run by comparing patch lists
/// before and after execution. Excludes automatic backup patches.
async fn collect_created_patches(
    harness: &TestHarness,
    before_patch_ids: &[PatchId],
) -> Result<Vec<PatchId>> {
    let client = harness.client()?;
    let after_patches = client
        .list_patches(&SearchPatchesQuery::default())
        .await
        .context("failed to list patches after worker run")?;

    let created = after_patches
        .patches
        .into_iter()
        .filter(|p| !p.patch.is_automatic_backup)
        .filter(|p| !before_patch_ids.contains(&p.patch_id))
        .map(|p| p.patch_id)
        .collect();

    Ok(created)
}

/// Get the current status of a job.
async fn get_session_status(harness: &TestHarness, job_id: &SessionId) -> Result<Status> {
    let client = harness.client()?;
    let jobs = client
        .list_sessions(&SearchSessionsQuery::default())
        .await
        .context("failed to list jobs for status check")?;
    let job = jobs
        .sessions
        .iter()
        .find(|j| &j.session_id == job_id)
        .with_context(|| format!("job '{job_id}' not found after worker run"))?;
    Ok(job.session.status)
}

/// Wait for a job to reach Running status, polling until the timeout.
async fn wait_for_running(harness: &TestHarness, job_id: &SessionId) -> Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if std::time::Instant::now() > deadline {
            bail!("timed out waiting for job '{job_id}' to reach Running status");
        }
        let status = get_session_status(harness, job_id).await?;
        if status == Status::Running {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

pub(super) async fn run_worker_impl(
    harness: &TestHarness,
    job_id: &SessionId,
    commands: Vec<&str>,
    fail_after_run: bool,
) -> Result<WorkerResult> {
    // Ensure env vars are set for the worker subprocess.
    ensure_worker_env_vars(harness, job_id).await?;

    // Wait for the job to reach Running status (background workers handle
    // the Created -> Pending -> Running transitions).
    wait_for_running(harness, job_id).await?;

    // Snapshot existing patches before the worker run.
    let client = harness.client()?;
    let before_patches = client
        .list_patches(&SearchPatchesQuery::default())
        .await
        .context("failed to list patches before worker run")?;
    let before_patch_ids: Vec<PatchId> = before_patches
        .patches
        .iter()
        .filter(|p| !p.patch.is_automatic_backup)
        .map(|p| p.patch_id.clone())
        .collect();

    // Create BashCommands and run the worker.
    let string_commands: Vec<String> = commands.iter().map(|s| s.to_string()).collect();
    let bash_commands = BashCommands::new(string_commands, fail_after_run);

    let temp_dir =
        tempfile::tempdir().context("failed to create temporary directory for worker")?;
    let worker_dir = temp_dir.path().to_path_buf();

    let context = CommandContext::new(ResolvedOutputFormat::Pretty);
    let run_result = metis::command::sessions::worker_run::run(
        harness.default_user().client(),
        job_id.clone(),
        worker_dir,
        None,
        true, // use_tempdir — matches production (K8s always passes --tempdir)
        &bash_commands,
        &context,
    )
    .await;

    let outputs = bash_commands.outputs();

    if let Err(err) = run_result {
        let formatted = format_command_outputs(&outputs);
        return Err(anyhow::anyhow!(
            "worker run failed: {err}\ncommand outputs:\n{formatted}"
        ));
    }

    let patches_created = collect_created_patches(harness, &before_patch_ids).await?;
    let final_status = get_session_status(harness, job_id).await?;

    Ok(WorkerResult {
        outputs,
        patches_created,
        final_status,
    })
}

pub(super) async fn run_worker_expect_failure_impl(
    harness: &TestHarness,
    job_id: &SessionId,
    commands: Vec<&str>,
) -> Result<WorkerFailure> {
    // Ensure env vars are set for the worker subprocess.
    ensure_worker_env_vars(harness, job_id).await?;

    // Wait for the job to reach Running status.
    wait_for_running(harness, job_id).await?;

    // Create BashCommands (not configured to fail — the commands themselves should fail).
    let string_commands: Vec<String> = commands.iter().map(|s| s.to_string()).collect();
    let bash_commands = BashCommands::new(string_commands, false);

    let temp_dir =
        tempfile::tempdir().context("failed to create temporary directory for worker")?;
    let worker_dir = temp_dir.path().to_path_buf();

    let context = CommandContext::new(ResolvedOutputFormat::Pretty);
    let run_result = metis::command::sessions::worker_run::run(
        harness.default_user().client(),
        job_id.clone(),
        worker_dir,
        None,
        true, // use_tempdir — matches production (K8s always passes --tempdir)
        &bash_commands,
        &context,
    )
    .await;

    let outputs = bash_commands.outputs();
    let final_status = get_session_status(harness, job_id).await?;

    match run_result {
        Ok(()) => Err(anyhow::anyhow!(
            "expected worker run to fail, but it succeeded"
        )),
        Err(error) => Ok(WorkerFailure {
            error,
            outputs,
            final_status,
        }),
    }
}

fn format_command_outputs(outputs: &[CommandOutput]) -> String {
    outputs
        .iter()
        .map(|output| {
            format!(
                "command: {command}\nstatus: {status}\nstdout:\n{stdout}\nstderr:\n{stderr}",
                command = output.command,
                status = output.status,
                stdout = output.stdout.trim_end(),
                stderr = output.stderr.trim_end(),
            )
        })
        .collect::<Vec<_>>()
        .join("\n---\n")
}
