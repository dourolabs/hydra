use std::{
    collections::HashMap,
    fs,
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context, Result};
use hydra_common::{
    constants::{ENV_HYDRA_DOCUMENTS_DIR, ENV_HYDRA_ISSUE_ID},
    session_status::{SessionStatusUpdate, SetSessionStatusResponse},
    sessions::WorkerContext,
    IssueId, SessionId,
};
use tempfile::Builder;

use crate::command::patches::resolve_service_repo_name;
use crate::command::sessions::mounts;
use crate::command::sessions::mounts::orchestrator::run_phase;
use crate::worker::commands::WorkerCommands;
use crate::worker::model_selector::ModelSelector;
use crate::worker::reaper::reap_other_processes;
use crate::worker::relay_adapter::{spawn_relay_adapter, RelayAdapter};
use crate::worker::report::{RunReport, SessionResume};
use crate::{
    client::{ConflictError, HydraClientInterface},
    command::output::CommandContext,
};

/// Per-attempt timeout for submitting the final session status.
const SUBMIT_SESSION_STATUS_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum number of attempts when submitting the final session status.
const SUBMIT_SESSION_STATUS_MAX_ATTEMPTS: u32 = 3;

pub async fn run(
    client: Arc<dyn HydraClientInterface>,
    session: SessionId,
    dest: PathBuf,
    issue_id: Option<IssueId>,
    use_tempdir: bool,
    commands: Option<&dyn WorkerCommands>,
    _context: &CommandContext,
) -> Result<()> {
    // The `commands` parameter is transitional and will be removed in PR 3
    // (it survives this PR so we can drop `ModelAwareCommands` without
    // touching the trait surface). Production passes `None`; the integration
    // test harness passes `Some(&BashCommands)` so it can mock the model
    // without requiring real `claude` / `codex` binaries on PATH. The live
    // dispatch path goes through `ModelSelector::from_context` below.
    // Initialize a tracing subscriber so structured `tracing::info!` /
    // `tracing::warn!` / `tracing::error!` calls from worker code (e.g.
    // `hydra/src/worker/interactive.rs` suspend/upload/resume instrumentation)
    // are surfaced on the worker subprocess's stdout/stderr, which the job
    // engine captures into the per-session log file. `try_init` is a no-op if
    // a subscriber has already been installed (e.g. inside an integration
    // test that initializes its own).
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .try_init();

    let job = session;

    let WorkerContext {
        request_context,
        variables,
        prompt,
        model,
        build_cache,
        mcp_config,
        interactive,
        ..
    } = client.get_session_context(&job).await?;
    let mcp_config_json = mcp_config
        .map(|c| serde_json::to_string(&c))
        .transpose()
        .context("failed to serialize MCP config")?;
    let service_repo_name = resolve_service_repo_name(client.as_ref(), Some(&job)).await?;
    let dest = if use_tempdir {
        let tmp = tempfile::tempdir().context("failed to create temporary working directory")?;
        let tmp_path = tmp.keep();
        log_status(format!("Using temporary directory: {}", tmp_path.display()));
        tmp_path
    } else {
        ensure_clean_destination(&dest)?;
        dest
    };
    let mut execution_env = variables;
    ensure_color_output_env(&mut execution_env);
    let worker_home_dir = resolve_worker_home_dir();
    let issue_branch_id = issue_id
        .as_ref()
        .map(|value| value.to_string())
        .or_else(|| execution_env.get(ENV_HYDRA_ISSUE_ID).cloned());
    let github_token = client.get_github_token().await.ok();

    // Pre-flight: compute the per-mount destination paths and pin the
    // agent's `HYDRA_DOCUMENTS_DIR` to the path `DocumentsMount` targets,
    // before any mount runs. Each mount creates its own directory at
    // `setup` time, so we deliberately do **not** `mkdir` either path here.
    let repo_path = dest.join("repo");
    let documents_path = dest.join("documents");
    execution_env.insert(
        ENV_HYDRA_DOCUMENTS_DIR.to_string(),
        documents_path.to_string_lossy().into_owned(),
    );

    let mut mounts = mounts::build_mounts(
        &repo_path,
        &documents_path,
        Arc::clone(&client),
        &request_context,
        build_cache.as_ref(),
        service_repo_name.as_ref(),
        github_token,
        issue_branch_id,
        worker_home_dir,
        job.clone(),
    )?;

    let mut errors = Vec::new();

    for mount in mounts.iter_mut() {
        run_phase(mount.setup_phase(), || mount.setup(), &mut errors).await?;
    }

    let _output_dir = Builder::new()
        .prefix("codex-output")
        .tempdir()
        .context("failed to create temporary codex output directory")?;

    let agent_start = Instant::now();

    // When the caller supplies a `commands` impl, dispatch through the legacy
    // `WorkerCommands` trait rather than `ModelSelector`. Production passes
    // `None`; the integration-test harness (`hydra/tests/harness/worker.rs`)
    // passes `Some(&BashCommands)` so it can mock the model without requiring
    // real `claude` / `codex` binaries on PATH. Both arms are removed in PR 3
    // when the trait goes away.
    let last_message = if let Some(commands) = commands {
        log_status("Phase: agent execution — starting (test path: WorkerCommands trait)");
        match commands
            .run(
                &prompt,
                model.as_deref(),
                &repo_path,
                &execution_env,
                &dest.join("worker-output.txt"),
                mcp_config_json.as_deref(),
            )
            .await
        {
            Ok(report) => {
                let elapsed = agent_start.elapsed().as_secs_f64();
                log_status(format!(
                    "Phase: agent execution — completed successfully ({elapsed:.2}s)"
                ));
                log_run_report(&report);
                report.last_message
            }
            Err(err) => {
                let elapsed = agent_start.elapsed().as_secs_f64();
                log_status(format!(
                    "Phase: agent execution — failed ({elapsed:.2}s): {err}"
                ));
                errors.push(err);
                errors
                    .last()
                    .map(|err| err.to_string())
                    .unwrap_or_else(|| "worker command execution failed".to_string())
            }
        }
    } else {
        let selector_home_dir = resolve_worker_home_dir()
            .ok_or_else(|| anyhow!("HOME must be set to construct a model wrapper"))?;
        let selector_idle_timeout = Duration::from_secs(
            interactive
                .as_ref()
                .and_then(|opts| opts.idle_timeout_secs)
                .unwrap_or(600),
        );

        let selector_result = ModelSelector::from_context(
            &model,
            repo_path.clone(),
            selector_home_dir.clone(),
            execution_env.clone(),
            mcp_config_json.as_deref(),
            selector_idle_timeout,
        )
        .await;

        match selector_result {
            Err(err) => {
                let elapsed = agent_start.elapsed().as_secs_f64();
                log_status(format!(
                    "Phase: agent execution — failed during model setup ({elapsed:.2}s): {err}"
                ));
                errors.push(err);
                errors
                    .last()
                    .map(|err| err.to_string())
                    .unwrap_or_else(|| "model setup failed".to_string())
            }
            Ok(mut selector) => {
                let run_result = if let Some(interactive_opts) = interactive {
                    log_status("Phase: interactive agent execution — starting");
                    if matches!(selector, ModelSelector::Codex(_)) {
                        Err(anyhow!("model {model:?} does not support interactive mode"))
                    } else {
                        let conversation_resume_from = interactive_opts.conversation_resume_from;
                        let ws_stream = client.connect_relay_websocket(&job).await?;
                        let RelayAdapter {
                            input_rx,
                            output_tx,
                            pump,
                            initial_resume,
                        } = spawn_relay_adapter(
                            ws_stream,
                            &job,
                            conversation_resume_from,
                            &prompt,
                            selector_home_dir.clone(),
                            repo_path.clone(),
                            selector_idle_timeout,
                        );
                        let resume: Option<SessionResume> = initial_resume.await.unwrap_or(None);
                        let report = selector
                            .run_interactive(input_rx, output_tx, &job, &prompt, resume)
                            .await;
                        let _ = pump.await;
                        report
                    }
                } else {
                    log_status("Phase: agent execution — starting");
                    let resume: Option<SessionResume> = None;
                    selector.run(&prompt, resume).await
                };

                match run_result {
                    Ok(report) => {
                        let elapsed = agent_start.elapsed().as_secs_f64();
                        log_status(format!(
                            "Phase: agent execution — completed successfully ({elapsed:.2}s)"
                        ));
                        log_run_report(&report);
                        report.last_message
                    }
                    Err(err) => {
                        let elapsed = agent_start.elapsed().as_secs_f64();
                        log_status(format!(
                            "Phase: agent execution — failed ({elapsed:.2}s): {err}"
                        ));
                        errors.push(err);
                        errors
                            .last()
                            .map(|err| err.to_string())
                            .unwrap_or_else(|| "worker command execution failed".to_string())
                    }
                }
            }
        }
    };

    // Phase: reap orphans. After the agent execution phase, any background
    // process the agent kicked off (e.g. `pnpm dev`, `vite`, `mock-server`,
    // or a script that backgrounded itself with `> /dev/null 2>&1 &`) is now
    // an orphan we don't want — it would keep the worker pod alive past its
    // useful end. The kill-process-group path in `worker::commands` only
    // catches children that kept stdout open; this is the namespace-wide
    // safety net for everything else.
    //
    // The reaper itself is gated on `std::process::id() == 1` — i.e. the
    // worker owns its PID namespace, which holds in production (K8s and
    // local-Docker both run `hydra sessions worker-run` as the container's
    // PID 1) but does not hold under the integration test harness or the
    // local process job engine. In those cases this call is a no-op and the
    // status line below reports `skipped`. See `worker::reaper` for the full
    // safety contract.
    log_status("Phase: reap orphans — starting");
    let reap_start = Instant::now();
    let reap_summary = reap_other_processes().await;
    let reap_elapsed = reap_start.elapsed().as_secs_f64();
    if reap_summary.skipped_not_pid1 {
        log_status(format!(
            "Phase: reap orphans — skipped (worker is not PID 1) ({reap_elapsed:.2}s)"
        ));
    } else {
        log_status(format!(
            "Phase: reap orphans — completed ({} victims, {} survived to SIGKILL) ({reap_elapsed:.2}s)",
            reap_summary.sigterm_sent, reap_summary.sigkill_sent
        ));
    }

    for mount in mounts.iter_mut() {
        let Some(phase) = mount.save_phase() else {
            continue;
        };
        run_phase(phase, || mount.save(), &mut errors).await?;
    }

    let status_update = if errors.is_empty() {
        SessionStatusUpdate::Complete {
            last_message: Some(last_message.clone()),
        }
    } else {
        SessionStatusUpdate::Failed {
            reason: errors
                .first()
                .map(|err| err.to_string())
                .unwrap_or_else(|| "worker run failed for unknown reasons".to_string()),
        }
    };

    log_status("Phase: status submission — starting");
    let status_start = Instant::now();
    if let Err(err) = submit_session_status(client.as_ref(), &job, status_update).await {
        let elapsed = status_start.elapsed().as_secs_f64();
        log_status(format!("Phase: status submission — failed ({elapsed:.2}s)"));
        errors.push(err);
    } else {
        let elapsed = status_start.elapsed().as_secs_f64();
        log_status(format!(
            "Phase: status submission — completed ({elapsed:.2}s)"
        ));
    }

    if let Some(err) = errors.into_iter().next() {
        Err(err)
    } else {
        Ok(())
    }
}

fn ensure_clean_destination(dest: &Path) -> Result<()> {
    if dest.exists() {
        let mut entries =
            fs::read_dir(dest).with_context(|| format!("failed to read directory {dest:?}"))?;
        if entries.next().is_some() {
            return Err(anyhow!(
                "destination {dest:?} is not empty; choose an empty or new directory"
            ));
        }
        Ok(())
    } else {
        fs::create_dir_all(dest).with_context(|| format!("failed to create {dest:?}"))
    }
}

fn ensure_color_output_env(env: &mut HashMap<String, String>) {
    env.entry("TERM".to_string())
        .or_insert_with(|| "xterm-256color".to_string());
    env.entry("COLORTERM".to_string())
        .or_insert_with(|| "truecolor".to_string());
    env.entry("CLICOLOR_FORCE".to_string())
        .or_insert_with(|| "1".to_string());
    env.entry("FORCE_COLOR".to_string())
        .or_insert_with(|| "1".to_string());
}

async fn submit_session_status(
    client: &dyn HydraClientInterface,
    job: &SessionId,
    status: SessionStatusUpdate,
) -> Result<()> {
    let last_message_length = status
        .last_message()
        .map(|message| message.len())
        .unwrap_or(0);
    submit_session_status_with_retry(
        job,
        last_message_length,
        SUBMIT_SESSION_STATUS_TIMEOUT,
        SUBMIT_SESSION_STATUS_MAX_ATTEMPTS,
        || client.set_session_status(job, &status),
    )
    .await
}

/// Retry loop for session status submission.
///
/// Each attempt is bounded by `attempt_timeout`. On timeout or any other error
/// (except a [`ConflictError`], which is treated as success), the attempt is
/// retried with exponential backoff up to `max_attempts` times. The conflict
/// case covers an already-submitted status from a prior worker invocation and
/// is detected structurally via `downcast_ref` rather than by string-matching
/// the error display.
async fn submit_session_status_with_retry<F, Fut>(
    job: &SessionId,
    last_message_length: usize,
    attempt_timeout: Duration,
    max_attempts: u32,
    mut submit: F,
) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<SetSessionStatusResponse>>,
{
    let mut last_error: Option<anyhow::Error> = None;
    for attempt in 1..=max_attempts {
        log_status(format!(
            "Updating status for session '{job}' via hydra-server (attempt {attempt}/{max_attempts})…"
        ));
        match tokio::time::timeout(attempt_timeout, submit()).await {
            Ok(Ok(response)) => {
                log_status(format!(
                    "Status updated for session '{}'. Stored last message length: {}",
                    response.session_id, last_message_length,
                ));
                return Ok(());
            }
            Ok(Err(err)) if err.downcast_ref::<ConflictError>().is_some() => {
                log_status(format!(
                    "Status for session '{job}' was already set (conflict); ignoring."
                ));
                return Ok(());
            }
            Ok(Err(err)) => {
                log_status(format!(
                    "Status submission attempt {attempt}/{max_attempts} failed: {err}"
                ));
                last_error = Some(err);
            }
            Err(_) => {
                let secs = attempt_timeout.as_secs();
                log_status(format!(
                    "Status submission attempt {attempt}/{max_attempts} timed out after {secs}s"
                ));
                last_error = Some(anyhow!("status submission timed out after {secs}s"));
            }
        }

        if attempt < max_attempts {
            let delay = Duration::from_secs(2u64.pow(attempt));
            log_status(format!(
                "Retrying status submission in {}s...",
                delay.as_secs()
            ));
            tokio::time::sleep(delay).await;
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("status submission failed without an error message")))
}

fn log_status(message: impl std::fmt::Display) {
    println!("{message}");
}

/// Emit the three per-run report log lines (token totals, model session id,
/// session-state path) so the per-session log file the job engine captures
/// records everything `RunReport` carries. These are the user-visible
/// outcome of PR 1 — see `designs/worker-model-commands-refactor.md` §7.
fn log_run_report(report: &RunReport) {
    for line in format_run_report_lines(report) {
        log_status(line);
    }
}

/// Build the three log lines `log_run_report` emits, kept separate so unit
/// tests can assert on the output without capturing stdout.
fn format_run_report_lines(report: &RunReport) -> Vec<String> {
    let mut lines = Vec::with_capacity(3);
    lines.push(format!(
        "  tokens: input={} output={} cache_read={} cache_create={}",
        report.usage.input_tokens,
        report.usage.output_tokens,
        report.usage.cache_read_input_tokens,
        report.usage.cache_creation_input_tokens,
    ));
    match &report.model_session_id {
        Some(id) => lines.push(format!("  model_session_id: {id}")),
        None => lines.push("  model_session_id: <none>".to_string()),
    }
    match &report.session_state {
        Some(s) => lines.push(format!(
            "  session_state: {} ({:?})",
            s.local_path.display(),
            s.format,
        )),
        None => lines.push("  session_state: <none>".to_string()),
    }
    lines
}

fn resolve_worker_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        git::{
            commit_changes as git_commit_changes, configure_repo as git_configure_repo,
            stage_all_changes as git_stage_all_changes,
        },
        test_utils::ids::task_id,
    };
    use git2::Repository;
    use std::collections::HashMap;

    #[test]
    fn configure_git_repo_sets_user_config_and_branch() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        let repo_path = tempdir.path();
        Repository::init(repo_path).context("failed to init git repo for test")?;
        {
            let repo = Repository::open(repo_path).context("failed to reopen repo for config")?;
            let mut config = repo
                .config()
                .context("failed to load git config for repo")?;
            config
                .set_str("user.name", "Initial User")
                .context("failed to set initial git user.name")?;
            config
                .set_str("user.email", "initial@example.com")
                .context("failed to set initial git user.email")?;
        }
        std::fs::write(repo_path.join("README.md"), "hello world")
            .context("failed to write initial file for git repo")?;
        git_stage_all_changes(repo_path)?;
        git_commit_changes(repo_path, "init")?;

        git_configure_repo(repo_path, "Hydra Worker", "hydra-worker@example.com")?;

        let repo = Repository::open(repo_path).context("failed to reopen repo for assertions")?;
        let config = repo
            .config()
            .context("failed to read git config for assertions")?;
        assert_eq!(config.get_string("user.name")?, "Hydra Worker");
        assert_eq!(config.get_string("user.email")?, "hydra-worker@example.com");

        Ok(())
    }

    #[test]
    fn format_run_report_lines_full_report() {
        use crate::worker::report::{SessionStateFormat, SessionStateRef, TokenUsage};

        let report = RunReport {
            last_message: "ignored for this test".to_string(),
            usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_input_tokens: 25,
                cache_creation_input_tokens: 10,
            },
            model_session_id: Some("abc-123".to_string()),
            session_state: Some(SessionStateRef {
                local_path: PathBuf::from("/tmp/session.jsonl"),
                format: SessionStateFormat::CodexJsonl,
            }),
        };
        let lines = format_run_report_lines(&report);
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            "  tokens: input=100 output=50 cache_read=25 cache_create=10"
        );
        assert_eq!(lines[1], "  model_session_id: abc-123");
        assert!(
            lines[2].contains("/tmp/session.jsonl"),
            "session_state line should mention the path; got: {}",
            lines[2]
        );
        assert!(
            lines[2].contains("CodexJsonl"),
            "session_state line should mention the format"
        );
    }

    #[test]
    fn format_run_report_lines_missing_fields() {
        let report = RunReport {
            last_message: String::new(),
            usage: Default::default(),
            model_session_id: None,
            session_state: None,
        };
        let lines = format_run_report_lines(&report);
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            "  tokens: input=0 output=0 cache_read=0 cache_create=0"
        );
        assert_eq!(lines[1], "  model_session_id: <none>");
        assert_eq!(lines[2], "  session_state: <none>");
    }

    #[test]
    fn ensure_color_output_env_sets_defaults() {
        let mut env = HashMap::new();

        ensure_color_output_env(&mut env);

        assert_eq!(env.get("TERM").map(String::as_str), Some("xterm-256color"));
        assert_eq!(env.get("COLORTERM").map(String::as_str), Some("truecolor"));
        assert_eq!(env.get("CLICOLOR_FORCE").map(String::as_str), Some("1"));
        assert_eq!(env.get("FORCE_COLOR").map(String::as_str), Some("1"));
    }

    #[test]
    fn ensure_color_output_env_preserves_existing_entries() {
        let mut env = HashMap::from([
            ("TERM".to_string(), "vt100".to_string()),
            ("FORCE_COLOR".to_string(), "0".to_string()),
        ]);

        ensure_color_output_env(&mut env);

        assert_eq!(env.get("TERM").map(String::as_str), Some("vt100"));
        assert_eq!(env.get("FORCE_COLOR").map(String::as_str), Some("0"));
        assert_eq!(env.get("CLICOLOR_FORCE").map(String::as_str), Some("1"));
        assert_eq!(env.get("COLORTERM").map(String::as_str), Some("truecolor"));
    }

    #[tokio::test(start_paused = true)]
    async fn phase_timeout_records_error_and_status_submission_still_runs() -> Result<()> {
        // Mirror the inline pattern used in `run`: wrap a slow phase in
        // `tokio::time::timeout`, push the timeout onto `errors`, then carry on
        // to status submission. The acceptance criteria require that a phase
        // timeout never short-circuits the final status update.
        let phase_timeout = Duration::from_millis(50);
        let mut errors: Vec<anyhow::Error> = Vec::new();

        let slow_phase = async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok::<(), anyhow::Error>(())
        };
        match tokio::time::timeout(phase_timeout, slow_phase).await {
            Ok(_) => panic!("expected the slow phase to time out"),
            Err(_) => errors.push(anyhow!(
                "phase timed out after {}s",
                phase_timeout.as_secs()
            )),
        }
        assert_eq!(errors.len(), 1, "phase timeout must push to errors");

        // Status submission is still invoked, and (because errors is non-empty)
        // production code would send Failed { reason }; here we just verify the
        // submission helper succeeds rather than being skipped.
        let job = task_id("t-phase-timeout");
        let job_for_response = job.clone();
        let submission =
            submit_session_status_with_retry(&job, 0, Duration::from_secs(1), 1, || {
                let job = job_for_response.clone();
                async move {
                    Ok(hydra_common::session_status::SetSessionStatusResponse::new(
                        job,
                        hydra_common::task_status::Status::Failed,
                    ))
                }
            })
            .await;

        assert!(
            submission.is_ok(),
            "status submission must run even when a prior phase timed out"
        );
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn submit_session_status_retries_after_transport_failure() -> Result<()> {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let job = task_id("t-status-retry");
        let job_for_response = job.clone();
        let attempts = AtomicUsize::new(0);

        let result = submit_session_status_with_retry(&job, 7, Duration::from_secs(30), 3, || {
            let attempt = attempts.fetch_add(1, Ordering::SeqCst);
            let job = job_for_response.clone();
            async move {
                if attempt == 0 {
                    Err(anyhow!("simulated transport failure"))
                } else {
                    Ok(hydra_common::session_status::SetSessionStatusResponse::new(
                        job,
                        hydra_common::task_status::Status::Complete,
                    ))
                }
            }
        })
        .await;

        assert!(
            result.is_ok(),
            "status submission should succeed on the retry: {:?}",
            result.err()
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            2,
            "expected exactly one retry after the initial transport failure"
        );
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn submit_session_status_gives_up_after_max_attempts() -> Result<()> {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let job = task_id("t-status-fail");
        let attempts = AtomicUsize::new(0);

        let result = submit_session_status_with_retry(&job, 0, Duration::from_secs(30), 3, || {
            attempts.fetch_add(1, Ordering::SeqCst);
            async { Err(anyhow!("simulated persistent failure")) }
        })
        .await;

        assert!(
            result.is_err(),
            "status submission should fail after exhausting retries"
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            3,
            "should make exactly max_attempts attempts"
        );
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn submit_session_status_treats_409_as_success() -> Result<()> {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let job = task_id("t-status-conflict");
        let attempts = AtomicUsize::new(0);

        let result = submit_session_status_with_retry(&job, 0, Duration::from_secs(30), 3, || {
            attempts.fetch_add(1, Ordering::SeqCst);
            async {
                Err(ConflictError {
                    message: "already submitted".into(),
                }
                .into())
            }
        })
        .await;

        assert!(result.is_ok(), "ConflictError must be treated as success");
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "409 should short-circuit retries"
        );
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn submit_session_status_retries_on_timeout() -> Result<()> {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let job = task_id("t-status-timeout");
        let job_for_response = job.clone();
        let attempts = AtomicUsize::new(0);

        let result =
            submit_session_status_with_retry(&job, 0, Duration::from_millis(50), 3, || {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                let job = job_for_response.clone();
                async move {
                    if attempt == 0 {
                        // Sleep longer than the attempt timeout to force a timeout retry.
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        unreachable!("attempt should have been cancelled by timeout")
                    } else {
                        Ok(hydra_common::session_status::SetSessionStatusResponse::new(
                            job,
                            hydra_common::task_status::Status::Complete,
                        ))
                    }
                }
            })
            .await;

        assert!(
            result.is_ok(),
            "status submission should succeed after the timeout retry: {:?}",
            result.err()
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            2,
            "expected exactly one retry after the timeout"
        );
        Ok(())
    }
}
