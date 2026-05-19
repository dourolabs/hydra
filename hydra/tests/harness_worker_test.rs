mod harness;

use anyhow::Result;
use hydra_common::task_status::Status;
use std::str::FromStr;

#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
use std::process::Stdio;
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use tokio::process::Command;

#[cfg(unix)]
use harness::RelayCallCountingClient;
#[cfg(unix)]
use hydra::client::HydraClientInterface;
#[cfg(unix)]
use hydra::command::output::{CommandContext, ResolvedOutputFormat};
#[cfg(unix)]
use hydra_common::{
    issues::{IssueStatus, IssueType, SessionSettings},
    sessions::{BundleSpec, CreateSessionRequest},
    SessionId,
};
#[cfg(unix)]
use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::sync::Arc;

/// Integration test: create issue -> create job -> run_worker with git commit
/// + patch create -> verify patch exists and job completes.
#[tokio::test]
async fn run_worker_creates_patch() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/worker-test")
        .build()
        .await?;
    let user = harness.default_user();

    let repo = hydra_common::RepoName::from_str("acme/worker-test")?;
    let issue_id = user.create_issue("worker patch integration test").await?;
    let job_id = user
        .create_session_for_issue(&repo, "worker patch integration test", &issue_id)
        .await?;

    let result = harness
        .run_worker(
            &job_id,
            vec![
                "echo 'worker content' >> README.md",
                "git add README.md",
                "git commit -m 'worker changes'",
                "hydra patches create --title 'harness worker patch' --description 'created by harness worker'",
            ],
        )
        .await?;

    assert_eq!(
        result.final_status,
        Status::Complete,
        "job should complete after successful worker run"
    );
    assert_eq!(
        result.patches_created.len(),
        1,
        "worker should create exactly one non-backup patch"
    );

    // Verify the patch content through the API.
    let patch = user.get_patch(&result.patches_created[0]).await?;
    assert_eq!(patch.patch.title, "harness worker patch");

    Ok(())
}

/// Verify that run_worker returns captured command outputs.
#[tokio::test]
async fn run_worker_captures_command_outputs() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/outputs-test")
        .build()
        .await?;
    let user = harness.default_user();

    let repo = hydra_common::RepoName::from_str("acme/outputs-test")?;
    let job_id = user.create_session(&repo, "output capture test").await?;

    let result = harness
        .run_worker(&job_id, vec!["echo hello world"])
        .await?;

    assert!(!result.outputs.is_empty(), "should have captured outputs");
    assert!(
        result.outputs[0].stdout.contains("hello world"),
        "captured stdout should contain echo output"
    );
    assert_eq!(result.outputs[0].status, 0, "echo should succeed");

    Ok(())
}

/// Regression guard for the worker_run::run reaper (see hydra/src/worker/reaper.rs).
///
/// The reaper SIGTERMs every other process in the namespace after the agent
/// phase. Inside a worker pod the worker is PID 1, so it only sees its own
/// children; inside cargo-nextest the worker is **not** PID 1, so reaping
/// indiscriminately would SIGTERM the test runner. The reaper gate
/// (`std::process::id() == 1`) makes it a no-op outside a worker container —
/// this test pins that behavior end-to-end by spawning a sentinel `sleep 60`
/// before the worker run and asserting it survives.
#[cfg(unix)]
#[tokio::test]
async fn run_worker_does_not_reap_test_runner_processes() -> Result<()> {
    let mut sentinel = Command::new("sleep")
        .arg("60")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn sentinel sleep");
    let sentinel_pid = sentinel.id().expect("sentinel should have a pid");
    // Give the kernel a beat to materialize /proc/<pid>.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        Path::new(&format!("/proc/{sentinel_pid}")).exists(),
        "sentinel pid {sentinel_pid} should exist before the worker run",
    );

    let harness = harness::TestHarness::builder()
        .with_repo("acme/reaper-noop-test")
        .build()
        .await?;
    let user = harness.default_user();
    let repo = hydra_common::RepoName::from_str("acme/reaper-noop-test")?;
    let job_id = user.create_session(&repo, "reaper noop test").await?;

    let _result = harness
        .run_worker(&job_id, vec!["echo reaper-noop"])
        .await?;

    // Real assertion: the sentinel must still be running. If the reaper had
    // run as if it owned the namespace, it would have SIGTERMed this PID.
    assert!(
        Path::new(&format!("/proc/{sentinel_pid}")).exists(),
        "sentinel pid {sentinel_pid} must survive worker_run::run — the reaper \
         must not fire in the cargo-nextest harness (it is not PID 1 here)",
    );

    let _ = sentinel.kill().await;
    Ok(())
}

/// Regression guard for the Codex + interactive short-circuit in
/// `worker_run::run` (see `hydra/src/command/sessions/worker_run.rs:213`).
///
/// The `matches!(selector, ModelSelector::Codex(_))` guard returns `Err`
/// *before* calling `client.connect_relay_websocket` at line 217 — so a
/// gpt-4o (Codex-class) model in interactive mode must never open a relay
/// websocket. This test pins that end-to-end by:
///
///   1. Creating an issue whose `session_settings.model` is `"gpt-4o"`.
///   2. Creating an interactive session linked to that issue, with
///      `BundleSpec::None` (no repo clone) and `OPENAI_API_KEY=test` in the
///      session variables so `Codex::new` env validation passes.
///   3. Writing a fake `codex` shell script (exit 0) to a TempDir and
///      prepending it to `PATH` so `Codex::new`'s `codex login --with-api-key`
///      subprocess succeeds without a real `codex` binary.
///   4. Wrapping the harness client in `RelayCallCountingClient`, which
///      intercepts `connect_relay_websocket` to increment a counter and
///      return `Err` (so a regression fails LOUDLY at the call site in
///      addition to the post-hoc counter assertion).
///   5. Calling `worker_run::run(...)` with `commands = None` so dispatch
///      goes through `ModelSelector::from_context` (the production path).
///
/// Asserts the call returns `Err` and the counter remained at 0.
#[cfg(unix)]
#[tokio::test]
async fn run_worker_gpt4o_interactive_rejects_before_opening_relay() -> Result<()> {
    // (2) Write a fake `codex` script to a TempDir, prepend it to PATH.
    let codex_dir = tempfile::tempdir().expect("failed to create tempdir for fake codex");
    let codex_path = codex_dir.path().join("codex");
    std::fs::write(&codex_path, "#!/usr/bin/env sh\nexit 0\n")
        .expect("failed to write fake codex script");
    let mut perms = std::fs::metadata(&codex_path)
        .expect("failed to stat fake codex script")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&codex_path, perms)
        .expect("failed to chmod fake codex script executable");
    // Prepend the tempdir to PATH so `codex login --with-api-key` resolves to
    // our no-op script. cargo nextest runs each test in its own process, so
    // mutating PATH here is safe.
    let existing_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{existing_path}", codex_dir.path().display());
    std::env::set_var("PATH", &new_path);

    // (1) Stand up the harness. No repo needed — the session uses BundleSpec::None.
    let harness = harness::TestHarness::builder().build().await?;
    let user = harness.default_user();

    // Create an issue whose session_settings carry the model name. The
    // server reads `model` off the issue's session_settings when creating
    // a session linked to the issue.
    let mut settings = SessionSettings::default();
    settings.model = Some("gpt-4o".to_string());
    let issue_id = user
        .create_issue_with_settings(
            "gpt-4o interactive regression guard",
            IssueType::Task,
            IssueStatus::Open,
            None,
            Some(settings),
        )
        .await?;

    // (3) Construct a CreateSessionRequest inline so we can set both
    // `BundleSpec::None` and `interactive = true`. The existing UserHandle
    // helpers force `BundleSpec::ServiceRepository`, which would require a
    // configured repo.
    let mut variables = HashMap::new();
    variables.insert("OPENAI_API_KEY".to_string(), "test".to_string());
    let create_request = CreateSessionRequest::new(
        "regression-guard prompt".to_string(),
        None,
        BundleSpec::None,
        variables,
        Some(issue_id.clone()),
        None,
        true, // interactive
    );
    let job_id = user.client().create_session(&create_request).await?.session_id;

    // (4) Wait for the session to reach Running status. The
    // `start_created_sessions` automation transitions Created → Pending,
    // and the MockJobEngine reports the job as Running, which the
    // monitor reconciles to the session.
    wait_for_session_running(&harness, &job_id).await?;

    // (5) Wrap the harness client in RelayCallCountingClient. The wrapper
    // forwards every call EXCEPT `connect_relay_websocket`, which it
    // counts and short-circuits with an Err.
    let inner: Arc<dyn HydraClientInterface> = Arc::new(user.client().clone());
    let wrapper = Arc::new(RelayCallCountingClient::new(inner));

    // (6) Invoke worker_run::run with commands = None so dispatch goes
    // through ModelSelector::from_context (the production path).
    let temp_dir =
        tempfile::tempdir().expect("failed to create temporary worker directory");
    let worker_dir = temp_dir.path().to_path_buf();
    let context = CommandContext::new(ResolvedOutputFormat::Pretty);
    let client_for_run: Arc<dyn HydraClientInterface> = wrapper.clone();
    let run_result = hydra::command::sessions::worker_run::run(
        client_for_run,
        job_id.clone(),
        worker_dir,
        None,
        true,
        None, // commands = None forces the ModelSelector branch
        &context,
    )
    .await;

    // (7) The run must fail — Codex + interactive returns Err at the
    // `matches!(selector, ModelSelector::Codex(_))` guard, before
    // `connect_relay_websocket` is called.
    assert!(
        run_result.is_err(),
        "worker_run::run must return Err for a Codex model + interactive session"
    );
    let err_message = run_result.unwrap_err().to_string();
    assert!(
        err_message.contains("interactive") || err_message.contains("does not support"),
        "expected error message to mention interactive mode, got: {err_message}"
    );

    // (8) And the relay websocket must never have been opened.
    assert_eq!(
        wrapper.relay_call_count(),
        0,
        "connect_relay_websocket must be invoked exactly 0 times — the \
         Codex+interactive guard at worker_run.rs:213 short-circuits before \
         the relay open at line 217"
    );

    Ok(())
}

#[cfg(unix)]
async fn wait_for_session_running(
    harness: &harness::TestHarness,
    job_id: &SessionId,
) -> Result<()> {
    use hydra_common::sessions::SearchSessionsQuery;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let client = harness.client()?;
    loop {
        if std::time::Instant::now() > deadline {
            anyhow::bail!("timed out waiting for session '{job_id}' to reach Running status");
        }
        let sessions = client.list_sessions(&SearchSessionsQuery::default()).await?;
        if let Some(record) = sessions.sessions.iter().find(|s| &s.session_id == job_id) {
            if record.session.status == Status::Running {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Verify that run_worker_expect_failure returns WorkerFailure when a command fails.
#[tokio::test]
async fn run_worker_expect_failure_captures_error() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/fail-test")
        .build()
        .await?;
    let user = harness.default_user();

    let repo = hydra_common::RepoName::from_str("acme/fail-test")?;
    let job_id = user.create_session(&repo, "failure test").await?;

    let failure = harness
        .run_worker_expect_failure(&job_id, vec!["exit 1"])
        .await?;

    assert_eq!(
        failure.final_status,
        Status::Failed,
        "job should be marked as failed after worker failure"
    );
    assert!(
        !failure.error.to_string().is_empty(),
        "failure should contain an error message"
    );

    Ok(())
}
