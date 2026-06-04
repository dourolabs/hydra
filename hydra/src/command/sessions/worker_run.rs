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
    api::v1::sessions::SessionModeKind,
    constants::{DEFAULT_CONVERSATION_TIMEOUT_SECS, ENV_HYDRA_DOCUMENTS_DIR, ENV_HYDRA_ISSUE_ID},
    session_status::{SessionStatusUpdate, SetSessionStatusResponse},
    sessions::{MountSpec, WorkerContext},
    SessionId,
};

use crate::command::sessions::mounts;
use crate::command::sessions::mounts::orchestrator::run_phase;
use crate::worker::model_selector::ModelSelector;
use crate::worker::reaper::reap_other_processes;
use crate::worker::relay_adapter::ReconnectFn;
use crate::worker::report::TokenUsage;
use crate::worker::socket::WorkerSocket;
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
    use_tempdir: bool,
    _context: &CommandContext,
) -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .try_init();

    let job = session;

    let WorkerContext {
        session_id: _session_id,
        mode_kind,
        mounts,
        working_dir,
        model,
        mcp_config,
        idle_timeout_secs,
        resolved_env,
        github_token,
        ..
    } = client.get_session_context(&job).await?;

    let mount_spec = MountSpec::new(working_dir, mounts);
    let mcp_config_json = mcp_config
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .context("failed to serialize MCP config")?;

    let dest = if use_tempdir {
        let tmp = tempfile::tempdir().context("failed to create temporary working directory")?;
        let tmp_path = tmp.keep();
        log_status(format!("Using temporary directory: {}", tmp_path.display()));
        tmp_path
    } else {
        ensure_clean_destination(&dest)?;
        dest
    };
    let mut execution_env = resolved_env;
    ensure_color_output_env(&mut execution_env);
    let worker_home_dir = resolve_worker_home_dir();

    if let Some(docs_target) = mounts::spec::find_documents_dir(&mount_spec) {
        execution_env.insert(
            ENV_HYDRA_DOCUMENTS_DIR.to_string(),
            dest.join(docs_target).to_string_lossy().into_owned(),
        );
    }

    let issue_branch_id = execution_env.get(ENV_HYDRA_ISSUE_ID).cloned();
    let mounts::spec::InstantiatedMounts {
        working_dir: repo_path,
        mounts: instantiated_mounts,
    } = mounts::spec::instantiate(
        &mount_spec,
        mounts::spec::InstantiateInputs {
            github_token: github_token.clone(),
            worker_home_dir: worker_home_dir.clone(),
            dest: &dest,
            client: Arc::clone(&client),
            session_id: job.clone(),
            issue_branch_id,
        },
    )
    .map_err(|err| anyhow!("failed to instantiate MountSpec: {err}"))?;
    let mut mounts = instantiated_mounts;

    let mut errors = Vec::new();

    for mount in mounts.iter_mut() {
        run_phase(mount.setup_phase(), || mount.setup(), &mut errors).await?;
    }

    let agent_start = Instant::now();

    let mut run_usage: Option<TokenUsage> = None;
    let interactive = matches!(mode_kind, SessionModeKind::Interactive);
    let last_message = if let Err(err) = reject_interactive_if_unsupported(&model, interactive) {
        let elapsed = agent_start.elapsed().as_secs_f64();
        log_status(format!(
            "Phase: agent execution — failed during model setup ({elapsed:.2}s): {err}"
        ));
        errors.push(err);
        errors
            .last()
            .map(|err| err.to_string())
            .unwrap_or_else(|| "model setup failed".to_string())
    } else {
        let selector_home_dir = worker_home_dir
            .clone()
            .ok_or_else(|| anyhow!("HOME must be set to construct a model wrapper"))?;
        let selector_idle_timeout =
            Duration::from_secs(idle_timeout_secs.unwrap_or(DEFAULT_CONVERSATION_TIMEOUT_SECS));

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
                let ws_stream = client.connect_relay_websocket(&job).await?;
                let ws = WorkerSocket::new(ws_stream);

                let run_result = if interactive {
                    log_status("Phase: interactive agent execution — starting");
                    let client_for_reconnect = Arc::clone(&client);
                    let reconnect: ReconnectFn<_> = Arc::new(move |sid| {
                        let client = client_for_reconnect.clone();
                        Box::pin(async move {
                            let stream = client.connect_relay_websocket(&sid).await?;
                            Ok(WorkerSocket::new(stream))
                        })
                    });
                    selector.drive_interactive(ws, job.clone(), reconnect).await
                } else {
                    log_status("Phase: agent execution — starting");
                    selector.drive_headless(ws).await
                };

                match run_result {
                    Ok(report) => {
                        let elapsed = agent_start.elapsed().as_secs_f64();
                        log_status(format!(
                            "Phase: agent execution — completed successfully ({elapsed:.2}s)"
                        ));
                        report.log();
                        run_usage = Some(report.usage.clone());
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
            usage: run_usage.clone(),
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

fn resolve_worker_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn reject_interactive_if_unsupported(model: &Option<String>, interactive: bool) -> Result<()> {
    if interactive && !ModelSelector::supports_interactive(model.as_deref()) {
        Err(anyhow!("model {model:?} does not support interactive mode"))
    } else {
        Ok(())
    }
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
        worker::report::RunReport,
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
    fn format_lines_full_report() {
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
        let lines = report.format_lines();
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            "  tokens: input=100 output=50 cache_read=25 cache_create=10 total=185"
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
    fn format_lines_missing_fields() {
        let report = RunReport {
            last_message: String::new(),
            usage: Default::default(),
            model_session_id: None,
            session_state: None,
        };
        let lines = report.format_lines();
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            "  tokens: input=0 output=0 cache_read=0 cache_create=0 total=0"
        );
        assert_eq!(lines[1], "  model_session_id: <none>");
        assert_eq!(lines[2], "  session_state: <none>");
    }

    #[test]
    fn reject_interactive_if_unsupported_codex_interactive_returns_err() {
        let model = Some("gpt-4o".to_string());
        let err = reject_interactive_if_unsupported(&model, true)
            .expect_err("Codex+interactive must be rejected");
        assert_eq!(
            err.to_string(),
            "model Some(\"gpt-4o\") does not support interactive mode"
        );
    }

    #[test]
    fn reject_interactive_if_unsupported_claude_interactive_returns_ok() {
        let model = Some("claude-3-5-sonnet".to_string());
        assert!(reject_interactive_if_unsupported(&model, true).is_ok());
    }

    #[test]
    fn reject_interactive_if_unsupported_codex_non_interactive_returns_ok() {
        let model = Some("gpt-4o".to_string());
        assert!(reject_interactive_if_unsupported(&model, false).is_ok());
    }

    #[test]
    fn reject_interactive_if_unsupported_none_interactive_returns_err() {
        let err = reject_interactive_if_unsupported(&None, true)
            .expect_err("None+interactive must be rejected");
        assert_eq!(
            err.to_string(),
            "model None does not support interactive mode"
        );
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

        assert!(result.is_ok());
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
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

        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
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

        assert!(result.is_ok());
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        Ok(())
    }
}
