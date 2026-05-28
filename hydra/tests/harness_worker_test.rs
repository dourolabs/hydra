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
    api::v1::{
        conversations::CreateConversationRequest,
        sessions::{AgentConfig, CreateSessionRequest, MountSpec, SessionMode},
    },
    issues::SessionSettings,
};
#[cfg(unix)]
use std::collections::HashMap;
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

/// Regression guard for the `ModelSelector::Codex` interactive-mode short-circuit
/// in `worker_run::run` (see `hydra/src/command/sessions/worker_run.rs`).
///
/// A Codex-class model selected for an interactive session must return `Err`
/// **before** any relay WebSocket is opened. Today's `ModelSelector::decide_kind`
/// unit tests cover the routing on paper; this test pins the invariant end-to-end
/// through the production dispatch path. The rejection is enforced by the
/// `reject_interactive_if_unsupported` fast-path, which fires before
/// `ModelSelector::from_context` runs any per-worker setup (e.g. `codex login`
/// or opening the relay WebSocket).
#[cfg(unix)]
#[tokio::test]
async fn run_worker_gpt4o_interactive_rejects_before_opening_relay() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    // Conversation carries `model = "gpt-4o"` in its session settings —
    // `gpt-4o` matches the `gpt-` prefix in `ModelSelector::decide_kind`,
    // routing to the Codex arm. For `SessionMode::Interactive`, the server
    // reads the model from the linked conversation's session_settings
    // (`CreateSessionRequest` carries no `model` field).
    let mut settings = SessionSettings::default();
    settings.model = Some("gpt-4o".to_string());
    let conversation = user
        .client()
        .create_conversation(&CreateConversationRequest {
            message: None,
            agent_name: None,
            session_settings: Some(settings),
        })
        .await?;
    let conversation_id = conversation.conversation_id;

    // Session: `SessionMode::Interactive` triggers the interactive branch in
    // worker_run; an empty MountSpec keeps mounts minimal (no clone, no
    // build cache).
    let request = CreateSessionRequest {
        mode: SessionMode::Interactive {
            conversation_id,
            idle_timeout_secs: None,
            greet_user: false,
        },
        agent_config: AgentConfig::default(),
        mount_spec: MountSpec::default(),
        image: None,
        env_vars: HashMap::new(),
        cpu_limit: None,
        memory_limit: None,
        secrets: None,
        spawned_from: None,
        resumed_from: None,
        initial_prompt: None,
    };
    let session_id = user.client().create_session(&request).await?.session_id;

    let inner: Arc<dyn HydraClientInterface> = Arc::new(user.client().clone());
    let wrapper = Arc::new(RelayCallCountingClient::new(inner));

    let temp_dir = tempfile::tempdir()?;
    let worker_dir = temp_dir.path().to_path_buf();
    let context = CommandContext::new(ResolvedOutputFormat::Pretty);

    let run_result = hydra::command::sessions::worker_run::run(
        wrapper.clone() as Arc<dyn HydraClientInterface>,
        session_id,
        worker_dir,
        true,
        &context,
    )
    .await;

    let err = run_result.expect_err("Codex+interactive must return Err");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("interactive") || msg.contains("does not support"),
        "expected error to mention the interactive guard, got: {msg}"
    );
    assert_eq!(
        wrapper.relay_call_count(),
        0,
        "connect_relay_websocket must not be invoked when the Codex interactive guard rejects",
    );

    Ok(())
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
