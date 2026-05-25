use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{anyhow, bail, Context, Result};
use hydra::command::sessions::mounts::{self, orchestrator::run_phase};
use hydra_common::{
    constants::{
        ENV_HYDRA_DOCUMENTS_DIR, ENV_HYDRA_ISSUE_ID, ENV_HYDRA_SERVER_URL, ENV_HYDRA_TOKEN,
    },
    patches::SearchPatchesQuery,
    session_status::SessionStatusUpdate,
    sessions::{SearchSessionsQuery, WorkerContext},
    task_status::Status,
    PatchId, SessionId,
};

use hydra_server::domain::actors::ActorRef;

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

/// Runs a shell command in the worker repo directory, replacing `hydra`
/// invocations with the test binary path. The result is appended to
/// `outputs` regardless of success.
async fn run_custom_command(
    command_string: &str,
    working_dir: &Path,
    env: &HashMap<String, String>,
    outputs: &Arc<Mutex<Vec<CommandOutput>>>,
) -> Result<CommandOutput> {
    let command_to_run = replace_hydra_in_command(command_string);

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
    outputs
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

fn hydra_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_hydra"))
}

/// Replace all command-position occurrences of `hydra` in a shell command
/// string with the test binary path. Handles compound commands joined by
/// `&&`, `||`, `;`, or `|`.
fn replace_hydra_in_command(command_string: &str) -> String {
    let hydra_path = hydra_bin();
    let hydra_path_str = hydra_path.to_string_lossy();
    let mut result = String::with_capacity(command_string.len());
    let mut remaining = command_string;

    // Process the first segment, then each subsequent segment after a shell operator.
    loop {
        let trimmed = remaining.trim_start();
        if trimmed.starts_with("hydra")
            && trimmed[5..]
                .chars()
                .next()
                .is_none_or(|c| c.is_whitespace())
        {
            // Preserve leading whitespace, then replace "hydra"
            let leading_ws = &remaining[..remaining.len() - trimmed.len()];
            result.push_str(leading_ws);
            result.push_str(&hydra_path_str);
            remaining = &trimmed[5..];
        }

        // Find the next shell operator
        if let Some((before, op, after)) = find_next_shell_operator(remaining) {
            result.push_str(before);
            result.push_str(op);
            remaining = after;
        } else {
            result.push_str(remaining);
            break;
        }
    }

    result
}

/// Find the next shell operator (`&&`, `||`, `;`, or `|`) in the string,
/// returning the text before it, the operator itself, and the text after.
/// Respects single and double quotes.
fn find_next_shell_operator(s: &str) -> Option<(&str, &str, &str)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'\'' {
                    i += 1;
                }
                i += 1; // skip closing quote
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1; // skip escaped char
                    }
                    i += 1;
                }
                i += 1; // skip closing quote
            }
            b'&' if i + 1 < bytes.len() && bytes[i + 1] == b'&' => {
                return Some((&s[..i], "&&", &s[i + 2..]));
            }
            b'|' if i + 1 < bytes.len() && bytes[i + 1] == b'|' => {
                return Some((&s[..i], "||", &s[i + 2..]));
            }
            b';' => {
                return Some((&s[..i], ";", &s[i + 1..]));
            }
            b'|' => {
                return Some((&s[..i], "|", &s[i + 1..]));
            }
            _ => {
                i += 1;
            }
        }
    }
    None
}

/// Ensure the job's environment variables include the server URL, auth
/// token, and issue ID so that subprocess commands (e.g. `hydra patches
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

    if !task.env_vars.contains_key(ENV_HYDRA_SERVER_URL) {
        task.env_vars
            .insert(ENV_HYDRA_SERVER_URL.to_string(), harness.server_url());
        changed = true;
    }
    if !task.env_vars.contains_key(ENV_HYDRA_TOKEN) {
        // Mint a job-scoped auth token so subprocess CLI calls present as a
        // session/issue actor, matching production. This is required for
        // server-side automations (e.g. LinkArtifactsToIssueAutomation) that
        // only fire for Session/Issue actors.
        let (_actor, auth_token) = harness
            .state()
            .create_actor_for_job(job_id.clone(), ActorRef::test())
            .await
            .context("failed to mint job-scoped auth token for worker subprocess")?;
        task.env_vars
            .insert(ENV_HYDRA_TOKEN.to_string(), auth_token);
        changed = true;
    }
    if !task.env_vars.contains_key(ENV_HYDRA_ISSUE_ID) {
        if let Some(issue_id) = &task.spawned_from {
            task.env_vars
                .insert(ENV_HYDRA_ISSUE_ID.to_string(), issue_id.to_string());
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

/// Drive the worker lifecycle for an integration test: mount setup,
/// bash-command "agent" phase, mount save, and final status submission.
///
/// This is a test-only replica of the production `worker_run::run` path,
/// substituting shell commands for the real `ModelSelector` dispatch so
/// tests do not need `claude` / `codex` on PATH. Patches are still created
/// by the bash commands themselves (typically via `hydra patches create`).
///
/// **Drift warning:** the orchestration glue here — phase order (mount
/// setup → agent → mount save → status submission), error-collection
/// semantics (`errors` collected across phases; first error becomes the
/// `Failed { reason }`; status submission still runs after a phase failed),
/// and final-status payload — duplicates the production flow in
/// `hydra/src/command/sessions/worker_run.rs::run`. Any change to that
/// lifecycle ordering needs a matching change here, or integration tests
/// will quietly diverge from production behavior. The duplication exists
/// because PR 3 removed the `WorkerCommands` mocking surface
/// (see `designs/worker-model-commands-refactor.md` §7), and the parent
/// design forbids new abstractions to bridge production and test paths.
async fn drive_worker_lifecycle(
    harness: &TestHarness,
    job_id: &SessionId,
    commands: &[String],
    fail_after_run: bool,
) -> (Vec<CommandOutput>, Result<()>) {
    let outputs: Arc<Mutex<Vec<CommandOutput>>> = Arc::new(Mutex::new(Vec::new()));

    let client: Arc<dyn hydra::client::HydraClientInterface> =
        Arc::new(harness.default_user().client().clone());

    let context = match client.get_session_context(job_id).await {
        Ok(ctx) => ctx,
        Err(err) => return (outputs.lock().unwrap().clone(), Err(err)),
    };
    let WorkerContext {
        session,
        resolved_env: mut variables,
        github_token,
        ..
    } = context;
    let mount_spec = session.mount_spec.clone();

    let temp_dir = match tempfile::tempdir() {
        Ok(t) => t,
        Err(err) => {
            return (
                outputs.lock().unwrap().clone(),
                Err(anyhow::Error::from(err).context("failed to create worker tempdir")),
            );
        }
    };
    let dest = temp_dir.path().to_path_buf();

    let worker_home_dir = std::env::var_os("HOME").map(PathBuf::from);

    if let Some(docs_target) = mounts::spec::find_documents_dir(&mount_spec) {
        variables.insert(
            ENV_HYDRA_DOCUMENTS_DIR.to_string(),
            dest.join(docs_target).to_string_lossy().into_owned(),
        );
    }
    let issue_branch_id = variables.get(ENV_HYDRA_ISSUE_ID).cloned();
    let (repo_path, mounts) = match mounts::spec::instantiate(
        &mount_spec,
        mounts::spec::InstantiateInputs {
            github_token: github_token.clone(),
            worker_home_dir: worker_home_dir.clone(),
            dest: &dest,
            client: Arc::clone(&client),
            session_id: job_id.clone(),
            issue_branch_id,
        },
    ) {
        Ok(mounts::spec::InstantiatedMounts {
            working_dir,
            mounts,
        }) => (working_dir, mounts),
        Err(err) => {
            return (
                outputs.lock().unwrap().clone(),
                Err(anyhow::anyhow!("failed to instantiate MountSpec: {err}")),
            );
        }
    };
    let mut mounts = mounts;

    let mut errors: Vec<anyhow::Error> = Vec::new();

    for mount in mounts.iter_mut() {
        if let Err(err) = run_phase(mount.setup_phase(), || mount.setup(), &mut errors).await {
            return (outputs.lock().unwrap().clone(), Err(err));
        }
    }

    let mut last_message = String::new();
    if errors.is_empty() {
        for command_string in commands {
            match run_custom_command(command_string, &repo_path, &variables, &outputs).await {
                Ok(output) => {
                    last_message = output.stdout;
                }
                Err(err) => {
                    errors.push(err.context(format!("failed to run command '{command_string}'")));
                    break;
                }
            }
        }
        if fail_after_run && errors.is_empty() {
            errors.push(anyhow!(
                "BashCommands configured to fail after running commands"
            ));
        }
    }

    for mount in mounts.iter_mut() {
        let Some(phase) = mount.save_phase() else {
            continue;
        };
        if let Err(err) = run_phase(phase, || mount.save(), &mut errors).await {
            return (outputs.lock().unwrap().clone(), Err(err));
        }
    }

    let status_update = if errors.is_empty() {
        SessionStatusUpdate::Complete {
            last_message: Some(last_message),
            usage: None,
        }
    } else {
        SessionStatusUpdate::Failed {
            reason: errors
                .first()
                .map(|err| err.to_string())
                .unwrap_or_else(|| "worker run failed for unknown reasons".to_string()),
        }
    };
    let _ = client.set_session_status(job_id, &status_update).await;

    let captured = outputs.lock().unwrap().clone();
    let result = if let Some(err) = errors.into_iter().next() {
        Err(err)
    } else {
        Ok(())
    };
    (captured, result)
}

pub(super) async fn run_worker_impl(
    harness: &TestHarness,
    job_id: &SessionId,
    commands: Vec<&str>,
    fail_after_run: bool,
) -> Result<WorkerResult> {
    ensure_worker_env_vars(harness, job_id).await?;
    wait_for_running(harness, job_id).await?;

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

    let string_commands: Vec<String> = commands.iter().map(|s| s.to_string()).collect();
    let (outputs, run_result) =
        drive_worker_lifecycle(harness, job_id, &string_commands, fail_after_run).await;

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
    ensure_worker_env_vars(harness, job_id).await?;
    wait_for_running(harness, job_id).await?;

    let string_commands: Vec<String> = commands.iter().map(|s| s.to_string()).collect();
    let (outputs, run_result) =
        drive_worker_lifecycle(harness, job_id, &string_commands, false).await;

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
