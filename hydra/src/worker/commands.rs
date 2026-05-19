//! Worker command trait (transitional).
//!
//! As of PR 2 of the worker model-commands refactor (see
//! `designs/worker-model-commands-refactor.md`) the live dispatch path goes
//! through [`crate::worker::model_selector::ModelSelector`]. This module
//! still exists to satisfy the `worker_run::run(commands: &dyn WorkerCommands, ...)`
//! parameter; PR 3 deletes it outright.
//!
//! The trait method bodies now return `Err` — they are never called in
//! production. `worker_run.rs` constructs a `ModelSelector` and dispatches
//! through it instead.

use std::{collections::HashMap, path::Path, time::Duration};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use hydra_common::SessionId;

use crate::client::RelayWebSocket;
use crate::worker::report::RunReport;

#[async_trait]
pub trait WorkerCommands: Send + Sync {
    async fn run(
        &self,
        prompt: &str,
        model: Option<&str>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
        mcp_config: Option<&str>,
    ) -> Result<RunReport>;

    /// Run a worker in interactive mode. Deprecated in favor of
    /// `ModelSelector::run_interactive`; see module-level docs.
    #[allow(clippy::too_many_arguments)]
    async fn run_interactive(
        &self,
        ws_stream: RelayWebSocket,
        session_id: &SessionId,
        prompt: &str,
        model: Option<&str>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        idle_timeout: Duration,
        conversation_resume_from: Option<usize>,
    ) -> Result<RunReport>;
}

pub struct ClaudeCommands;
pub struct CodexCommands;

pub struct ModelAwareCommands {
    #[allow(dead_code)]
    codex: CodexCommands,
    #[allow(dead_code)]
    claude: ClaudeCommands,
}

impl Default for ModelAwareCommands {
    fn default() -> Self {
        Self {
            codex: CodexCommands,
            claude: ClaudeCommands,
        }
    }
}

#[allow(dead_code)]
fn is_claude_model(model: &str) -> bool {
    let lc = model.to_ascii_lowercase();
    lc.contains("claude") || lc.contains("haiku") || lc.contains("sonnet") || lc.contains("opus")
}

#[async_trait]
impl WorkerCommands for ClaudeCommands {
    async fn run(
        &self,
        _prompt: &str,
        _model: Option<&str>,
        _working_dir: &Path,
        _env: &HashMap<String, String>,
        _output_path: &Path,
        _mcp_config: Option<&str>,
    ) -> Result<RunReport> {
        Err(anyhow!(
            "ClaudeCommands::run is deprecated; use ModelSelector::run via worker_run"
        ))
    }

    async fn run_interactive(
        &self,
        _ws_stream: RelayWebSocket,
        _session_id: &SessionId,
        _prompt: &str,
        _model: Option<&str>,
        _working_dir: &Path,
        _env: &HashMap<String, String>,
        _idle_timeout: Duration,
        _conversation_resume_from: Option<usize>,
    ) -> Result<RunReport> {
        Err(anyhow!(
            "ClaudeCommands::run_interactive is deprecated; use ModelSelector::run_interactive via worker_run"
        ))
    }
}

#[async_trait]
impl WorkerCommands for CodexCommands {
    async fn run(
        &self,
        _prompt: &str,
        _model: Option<&str>,
        _working_dir: &Path,
        _env: &HashMap<String, String>,
        _output_path: &Path,
        _mcp_config: Option<&str>,
    ) -> Result<RunReport> {
        Err(anyhow!(
            "CodexCommands::run is deprecated; use ModelSelector::run via worker_run"
        ))
    }

    async fn run_interactive(
        &self,
        _ws_stream: RelayWebSocket,
        _session_id: &SessionId,
        _prompt: &str,
        _model: Option<&str>,
        _working_dir: &Path,
        _env: &HashMap<String, String>,
        _idle_timeout: Duration,
        _conversation_resume_from: Option<usize>,
    ) -> Result<RunReport> {
        Err(anyhow!("interactive mode is not supported for Codex"))
    }
}

#[async_trait]
impl WorkerCommands for ModelAwareCommands {
    async fn run(
        &self,
        _prompt: &str,
        _model: Option<&str>,
        _working_dir: &Path,
        _env: &HashMap<String, String>,
        _output_path: &Path,
        _mcp_config: Option<&str>,
    ) -> Result<RunReport> {
        Err(anyhow!(
            "ModelAwareCommands::run is deprecated; use ModelSelector::run via worker_run"
        ))
    }

    async fn run_interactive(
        &self,
        _ws_stream: RelayWebSocket,
        _session_id: &SessionId,
        _prompt: &str,
        _model: Option<&str>,
        _working_dir: &Path,
        _env: &HashMap<String, String>,
        _idle_timeout: Duration,
        _conversation_resume_from: Option<usize>,
    ) -> Result<RunReport> {
        Err(anyhow!(
            "ModelAwareCommands::run_interactive is deprecated; use ModelSelector::run_interactive via worker_run"
        ))
    }
}
