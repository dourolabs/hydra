use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::Arc,
};

use anyhow::{bail, Context, Result};
use hydra::command::output::{CommandContext, ResolvedOutputFormat};
use hydra_common::{
    constants::{ENV_HYDRA_ISSUE_ID, ENV_HYDRA_SERVER_URL, ENV_HYDRA_TOKEN},
    patches::SearchPatchesQuery,
    sessions::SearchSessionsQuery,
    task_status::Status,
    PatchId, SessionId,
};
use tempfile::TempDir;

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

/// Test fixture standing in for the real `claude` binary.
///
/// Materializes a `claude` shim script in a temp dir, prepends that dir to
/// PATH (via the session's env_vars), and writes the test's shell commands
/// to a file the shim reads when invoked. Outputs are captured per-command
/// into the temp dir and read back via [`Self::read_outputs`].
struct FakeClaude {
    _temp_dir: TempDir,
    bin_dir: PathBuf,
    commands_file: PathBuf,
    outputs_dir: PathBuf,
    commands: Vec<String>,
    fail_after_run: bool,
}

impl FakeClaude {
    fn new(commands: Vec<String>, fail_after_run: bool) -> Result<Self> {
        let temp_dir = tempfile::tempdir().context("create fake-claude temp dir")?;
        let bin_dir = temp_dir.path().join("bin");
        let outputs_dir = temp_dir.path().join("outputs");
        fs::create_dir_all(&bin_dir).context("create fake-claude bin dir")?;
        fs::create_dir_all(&outputs_dir).context("create fake-claude outputs dir")?;

        // Substitute `hydra` for the test binary path before writing to the
        // commands file — the fake-claude shim doesn't know about cargo's
        // bin layout and just runs each line via `bash -c`.
        let substituted: Vec<String> = commands
            .into_iter()
            .map(|c| replace_hydra_in_command(&c))
            .collect();
        let commands_file = temp_dir.path().join("commands.txt");
        fs::write(&commands_file, substituted.join("\n"))
            .context("write fake-claude commands file")?;

        let script_path = bin_dir.join("claude");
        fs::write(&script_path, include_str!("fake_claude.sh"))
            .context("write fake-claude shim")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script_path)
                .context("stat fake-claude shim")?
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms)
                .context("chmod fake-claude shim")?;
        }

        Ok(Self {
            _temp_dir: temp_dir,
            bin_dir,
            commands_file,
            outputs_dir,
            commands: substituted,
            fail_after_run,
        })
    }

    /// Env vars the session must carry so the worker (a) finds the shim on
    /// PATH, (b) passes `Claude::new`'s API-key validation, and (c) tells
    /// the shim where its commands and outputs live.
    fn session_env_vars(&self) -> HashMap<String, String> {
        let parent_path = std::env::var("PATH").unwrap_or_default();
        let new_path = if parent_path.is_empty() {
            self.bin_dir.display().to_string()
        } else {
            format!("{}:{}", self.bin_dir.display(), parent_path)
        };
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), new_path);
        env.insert("ANTHROPIC_API_KEY".to_string(), "fake-claude-test".to_string());
        env.insert(
            "HYDRA_TEST_FAKE_CLAUDE_COMMANDS_FILE".to_string(),
            self.commands_file.display().to_string(),
        );
        env.insert(
            "HYDRA_TEST_FAKE_CLAUDE_OUTPUTS_DIR".to_string(),
            self.outputs_dir.display().to_string(),
        );
        if self.fail_after_run {
            env.insert(
                "HYDRA_TEST_FAKE_CLAUDE_FAIL_AFTER_RUN".to_string(),
                "1".to_string(),
            );
        }
        env
    }

    /// Read the per-command outputs the shim wrote. Stops at the first
    /// missing index (commands after a failure are never written).
    fn read_outputs(&self) -> Vec<CommandOutput> {
        let mut outputs = Vec::new();
        for idx in 0..self.commands.len() {
            let cmd_dir = self.outputs_dir.join(idx.to_string());
            if !cmd_dir.exists() {
                break;
            }
            let command = fs::read_to_string(cmd_dir.join("command")).unwrap_or_default();
            let stdout = fs::read_to_string(cmd_dir.join("stdout")).unwrap_or_default();
            let stderr = fs::read_to_string(cmd_dir.join("stderr")).unwrap_or_default();
            let status = fs::read_to_string(cmd_dir.join("status"))
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(-1);
            outputs.push(CommandOutput {
                command,
                stdout,
                stderr,
                status,
            });
        }
        outputs
    }
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

/// Ensure the job's environment variables, model selector, and auth token
/// are configured so the worker subprocess can (a) find the fake-claude
/// shim on PATH, (b) reach the test server, and (c) resolve its issue id.
async fn configure_worker_session(
    harness: &TestHarness,
    job_id: &SessionId,
    fake_claude: &FakeClaude,
) -> Result<()> {
    let store = harness.store();
    let versioned_task = store
        .get_session(job_id, false)
        .await
        .context("failed to get task to configure worker session")?;
    let mut task = versioned_task.item;

    // Force ModelSelector to pick the Claude path so the fake-claude shim
    // is invoked. `decide_kind` matches any model name containing "claude".
    task.model = Some("claude-fake".to_string());

    for (key, value) in fake_claude.session_env_vars() {
        task.env_vars.insert(key, value);
    }

    if !task.env_vars.contains_key(ENV_HYDRA_SERVER_URL) {
        task.env_vars
            .insert(ENV_HYDRA_SERVER_URL.to_string(), harness.server_url());
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
    }
    if !task.env_vars.contains_key(ENV_HYDRA_ISSUE_ID) {
        if let Some(issue_id) = &task.spawned_from {
            task.env_vars
                .insert(ENV_HYDRA_ISSUE_ID.to_string(), issue_id.to_string());
        }
    }

    store
        .update_session(job_id, task, &ActorRef::test())
        .await
        .context("failed to update task for worker")?;

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
    let string_commands: Vec<String> = commands.iter().map(|s| s.to_string()).collect();
    let fake_claude = FakeClaude::new(string_commands, fail_after_run)?;

    configure_worker_session(harness, job_id, &fake_claude).await?;

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

    let temp_dir =
        tempfile::tempdir().context("failed to create temporary directory for worker")?;
    let worker_dir = temp_dir.path().to_path_buf();

    let context = CommandContext::new(ResolvedOutputFormat::Pretty);
    let client: Arc<dyn hydra::client::HydraClientInterface> =
        Arc::new(harness.default_user().client().clone());
    let run_result = hydra::command::sessions::worker_run::run(
        client,
        job_id.clone(),
        worker_dir,
        None,
        true, // use_tempdir — matches production (K8s always passes --tempdir)
        &context,
    )
    .await;

    let outputs = fake_claude.read_outputs();

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
    let string_commands: Vec<String> = commands.iter().map(|s| s.to_string()).collect();
    let fake_claude = FakeClaude::new(string_commands, false)?;

    configure_worker_session(harness, job_id, &fake_claude).await?;

    // Wait for the job to reach Running status.
    wait_for_running(harness, job_id).await?;

    let temp_dir =
        tempfile::tempdir().context("failed to create temporary directory for worker")?;
    let worker_dir = temp_dir.path().to_path_buf();

    let context = CommandContext::new(ResolvedOutputFormat::Pretty);
    let client: Arc<dyn hydra::client::HydraClientInterface> =
        Arc::new(harness.default_user().client().clone());
    let run_result = hydra::command::sessions::worker_run::run(
        client,
        job_id.clone(),
        worker_dir,
        None,
        true, // use_tempdir — matches production (K8s always passes --tempdir)
        &context,
    )
    .await;

    let outputs = fake_claude.read_outputs();
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
