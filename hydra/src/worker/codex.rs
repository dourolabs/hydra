//! `Codex` per-worker wrapper. Owns env validation, `codex login`, MCP→TOML
//! config-file management, and the per-worker output tempdir used to capture
//! the `codex exec --json` JSONL stream. Constructed once at the top of
//! `worker_run::run` and reused across the worker's lifetime; the wrapper
//! speaks only Codex's native vocabulary (`codex exec --json` invocation,
//! Codex JSONL events) — the WS protocol is translated by
//! [`crate::worker::ModelSelector`].

use std::{collections::HashMap, path::PathBuf, process::Stdio};

use anyhow::{anyhow, Context, Result};
use hydra_common::constants::ENV_OPENAI_API_KEY;
use serde::Deserialize;
use serde_json::Value;
use tempfile::TempDir;
use tokio::{
    fs,
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::mpsc,
};

use crate::worker::report::{
    MaterializeError, NativeResume, RunReport, SessionStateFormat, SessionStateRef, TokenUsage,
    WorkerEvent,
};

/// Native resume handle for Codex. Placeholder today: Codex native resume
/// is out of scope (only Claude implements `try_materialize`), but the
/// variant must exist so the dispatcher's [`NativeResume`] enum is uniform
/// across wrappers and so a future implementation has a name to slot into.
#[derive(Debug, Clone)]
pub enum CodexResume {}

/// Per-worker Codex wrapper. Holds all state that does not need to change
/// between batch invocations (auth, on-disk config, output dir).
pub struct Codex {
    model: Option<String>,
    working_dir: PathBuf,
    env: HashMap<String, String>,
    /// Tracks the on-disk `~/.codex/config.toml` we wrote during construction
    /// (if any), so it is removed when `self` drops.
    _config_guard: Option<CodexConfigGuard>,
    /// Per-worker tempdir that receives the tee'd `codex exec --json` stdout
    /// as `session.jsonl`. Kept on `self` so the directory survives the
    /// duration of every `run()` call and the `session_state` path the report
    /// surfaces remains valid until the worker exits.
    output_dir: TempDir,
    /// Always `true` today; explicit so we do not lose the dimension when the
    /// design eventually adds a sandboxed run mode.
    #[allow(dead_code)]
    bypass_sandbox: bool,
}

impl Codex {
    /// Construct a new per-worker Codex wrapper.
    ///
    /// Performs the per-worker side effects up front so `run()` can be called
    /// any number of times against the same wrapper without re-running setup:
    ///   * validates `OPENAI_API_KEY` is present in `env`,
    ///   * runs `codex login --with-api-key` once,
    ///   * writes `<home>/.codex/config.toml` if an MCP config JSON was given
    ///     (cleaned up on `Drop`),
    ///   * creates the per-worker output tempdir used to capture
    ///     `codex exec --json` stdout.
    pub async fn new(
        model: Option<String>,
        working_dir: PathBuf,
        home_dir: PathBuf,
        env: HashMap<String, String>,
        mcp_config_json: Option<&str>,
    ) -> Result<Self> {
        let openai_api_key = env
            .get(ENV_OPENAI_API_KEY)
            .map(|s| s.as_str())
            .ok_or_else(|| {
                anyhow!("{ENV_OPENAI_API_KEY} must be provided via --openai-api-key or environment")
            })?;
        login(openai_api_key).await?;

        let config_guard = match mcp_config_json {
            Some(json) => Some(
                write_codex_mcp_config(&home_dir, json)
                    .await
                    .context("failed to write Codex MCP config")?,
            ),
            None => None,
        };

        let output_dir = tempfile::Builder::new()
            .prefix("codex-session")
            .tempdir()
            .context("failed to create codex output tempdir")?;

        Ok(Self {
            model,
            working_dir,
            env,
            _config_guard: config_guard,
            output_dir,
            bypass_sandbox: true,
        })
    }

    /// Attempt to materialize a resume blob into a native Codex resume
    /// handle. Codex native resume is unimplemented; this always returns
    /// [`MaterializeError::NotImplemented`] and the dispatcher falls back to
    /// transcript replay (which the dispatcher treats identically to every
    /// other `Err` variant).
    pub fn try_materialize(&self, _state_bytes: &[u8]) -> Result<NativeResume, MaterializeError> {
        Err(MaterializeError::NotImplemented)
    }

    /// Run one `codex exec --json` invocation and return its `RunReport`.
    ///
    /// If `event_tx` is `Some`, parsed agent-message items are translated to
    /// [`WorkerEvent::AssistantText`] and forwarded on it as they arrive so the
    /// headless dispatcher can stream per-step events to the server. The
    /// sender is dropped when the JSONL pump completes.
    pub async fn run(
        &mut self,
        prompt: &str,
        event_tx: Option<mpsc::Sender<WorkerEvent>>,
    ) -> Result<RunReport> {
        let session_log_path = self.output_dir.path().join("session.jsonl");

        let mut command = Command::new("codex");
        command
            .args([
                "exec",
                "--skip-git-repo-check",
                "--dangerously-bypass-approvals-and-sandbox",
                "--json",
            ])
            .current_dir(&self.working_dir)
            .envs(&self.env);
        #[cfg(unix)]
        command.process_group(0);
        if let Some(model) = self.model.as_deref() {
            command.arg("--model");
            command.arg(model);
        }
        command.arg("--");
        command.arg(prompt);

        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("failed to spawn codex command")?;
        let child_stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("failed to capture stdout for codex command"))?;

        let log_path = session_log_path.clone();
        let parse_handle = tokio::spawn(async move {
            let mut event_tx = event_tx;
            let mut log_file = fs::File::create(&log_path)
                .await
                .with_context(|| format!("failed to create codex session log {log_path:?}"))?;
            let mut reader = BufReader::new(child_stdout);
            let mut line = String::new();
            let mut state = CodexParseState::default();
            loop {
                line.clear();
                let read = reader
                    .read_line(&mut line)
                    .await
                    .context("failed to read codex stdout")?;
                if read == 0 {
                    break;
                }
                log_file
                    .write_all(line.as_bytes())
                    .await
                    .context("failed to write to codex session log")?;
                let mut stdout_writer = io::stdout();
                let _ = stdout_writer.write_all(line.as_bytes()).await;
                let _ = stdout_writer.flush().await;

                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<CodexEvent>(trimmed) {
                    Ok(event) => {
                        if let Some(tx) = event_tx.as_ref() {
                            if let Some(we) = event.to_worker_event() {
                                if tx.send(we).await.is_err() {
                                    event_tx = None;
                                }
                            }
                        }
                        state.apply(event);
                    }
                    Err(_) => {
                        tracing::warn!(line = %trimmed, "unparseable codex --json event");
                    }
                }
            }
            log_file
                .flush()
                .await
                .context("failed to flush codex session log")?;
            Ok::<CodexParseState, anyhow::Error>(state)
        });

        let status = child
            .wait()
            .await
            .context("failed waiting for codex command to finish")?;
        let state = parse_handle
            .await
            .context("failed to join codex stdout parser")??;

        if !status.success() {
            return Err(anyhow!("codex command failed with status {status}"));
        }

        let session_state =
            session_state_if_exists(session_log_path, SessionStateFormat::CodexJsonl);

        Ok(RunReport {
            last_message: state.last_message.unwrap_or_default(),
            usage: state.usage,
            model_session_id: state.session_id,
            session_state,
        })
    }
}

/// Run `codex login --with-api-key`, piping the OpenAI API key on stdin.
async fn login(openai_api_key: &str) -> Result<()> {
    let mut login_cmd = Command::new("codex")
        .args(["login", "--with-api-key"])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn codex login")?;

    {
        let mut stdin = login_cmd
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open stdin for codex login"))?;
        stdin
            .write_all(format!("{openai_api_key}\n").as_bytes())
            .await
            .with_context(|| format!("failed to write {ENV_OPENAI_API_KEY} to codex login"))?;
    }

    let status = login_cmd
        .wait()
        .await
        .context("failed waiting for codex login to finish")?;
    if !status.success() {
        return Err(anyhow!("codex login failed with status {status}"));
    }
    Ok(())
}

/// Writes `<home>/.codex/config.toml` with MCP server config and returns a
/// guard that removes the file (and the `.codex` dir if we created it) when
/// dropped.
async fn write_codex_mcp_config(
    home_dir: &std::path::Path,
    mcp_config: &str,
) -> Result<CodexConfigGuard> {
    let codex_dir = home_dir.join(".codex");
    let config_path = codex_dir.join("config.toml");
    let toml_content = mcp_config_to_codex_toml(mcp_config)?;

    let created_dir = !codex_dir.exists();
    if created_dir {
        fs::create_dir_all(&codex_dir)
            .await
            .with_context(|| format!("failed to create {codex_dir:?}"))?;
    }

    fs::write(&config_path, &toml_content)
        .await
        .with_context(|| format!("failed to write {config_path:?}"))?;

    Ok(CodexConfigGuard {
        config_path,
        codex_dir,
        created_dir,
    })
}

/// RAII guard for `<home>/.codex/config.toml` written for a Codex run.
///
/// Cleanup runs synchronously on `Drop` — async cleanup is not possible
/// in `Drop`; any best-effort kill / unlink must complete in the
/// destructor's synchronous frame.
pub(crate) struct CodexConfigGuard {
    config_path: PathBuf,
    codex_dir: PathBuf,
    created_dir: bool,
}

impl Drop for CodexConfigGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.config_path);
        if self.created_dir {
            let _ = std::fs::remove_dir(&self.codex_dir);
        }
    }
}

/// Converts Claude MCP JSON config to Codex TOML config format.
///
/// Input (Claude format):
///   {"mcpServers": {"name": {"command": "...", "args": [...], "env": {...}}}}
///
/// Output (Codex format):
///   [mcp_servers.name]
///   command = "..."
///   args = [...]
///   env = { KEY = "VALUE" }
pub(crate) fn mcp_config_to_codex_toml(mcp_json: &str) -> Result<String> {
    let parsed: Value =
        serde_json::from_str(mcp_json).context("failed to parse MCP config JSON")?;
    let servers = parsed
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow!("MCP config missing 'mcpServers' object"))?;

    let mut toml_table = toml::map::Map::new();
    let mut mcp_servers = toml::map::Map::new();

    for (name, server) in servers {
        let server_obj = server
            .as_object()
            .ok_or_else(|| anyhow!("MCP server '{name}' is not an object"))?;
        let mut entry = toml::map::Map::new();

        if let Some(command) = server_obj.get("command").and_then(|v| v.as_str()) {
            entry.insert(
                "command".to_string(),
                toml::Value::String(command.to_string()),
            );
        }

        if let Some(args) = server_obj.get("args").and_then(|v| v.as_array()) {
            let toml_args: Vec<toml::Value> = args
                .iter()
                .filter_map(|a| a.as_str().map(|s| toml::Value::String(s.to_string())))
                .collect();
            entry.insert("args".to_string(), toml::Value::Array(toml_args));
        }

        if let Some(env) = server_obj.get("env").and_then(|v| v.as_object()) {
            let mut env_table = toml::map::Map::new();
            for (k, v) in env {
                if let Some(val) = v.as_str() {
                    env_table.insert(k.clone(), toml::Value::String(val.to_string()));
                }
            }
            entry.insert("env".to_string(), toml::Value::Table(env_table));
        }

        mcp_servers.insert(name.clone(), toml::Value::Table(entry));
    }

    toml_table.insert("mcp_servers".to_string(), toml::Value::Table(mcp_servers));

    toml::to_string(&toml_table).context("failed to serialize Codex config to TOML")
}

/// Wrap a candidate session-state path into `Some(SessionStateRef)` iff it
/// exists on disk; return `None` otherwise (and log at debug). Mirrors the
/// helper in `claude.rs`.
fn session_state_if_exists(
    local_path: PathBuf,
    format: SessionStateFormat,
) -> Option<SessionStateRef> {
    if local_path.exists() {
        Some(SessionStateRef { local_path, format })
    } else {
        tracing::debug!(
            path = %local_path.display(),
            ?format,
            "session-state path does not exist; returning None"
        );
        None
    }
}

/// In-memory state accumulated while parsing the `codex exec --json` JSONL
/// stream.
#[derive(Default)]
struct CodexParseState {
    usage: TokenUsage,
    session_id: Option<String>,
    last_message: Option<String>,
}

impl CodexParseState {
    fn apply(&mut self, event: CodexEvent) {
        match event {
            CodexEvent::ThreadStarted { thread_id } => {
                self.session_id = Some(thread_id);
            }
            CodexEvent::ThreadTokenUsageUpdated { token_usage } => {
                self.usage = TokenUsage {
                    input_tokens: token_usage.input_tokens,
                    output_tokens: token_usage.output_tokens,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                };
            }
            CodexEvent::ItemCompleted { item } => {
                if let CodexItem::AgentMessage { text } = item {
                    self.last_message = Some(text);
                }
            }
            CodexEvent::Other => {}
        }
    }
}

/// Minimal parser for the subset of `codex exec --json` events the worker
/// cares about. Variant names are the wire-format strings emitted by
/// `codex 0.130` as `{"type":"..."}` tags. Any other event falls into
/// `Other` via `#[serde(other)]`.
#[derive(Deserialize)]
#[serde(tag = "type")]
enum CodexEvent {
    #[serde(rename = "thread.started")]
    ThreadStarted { thread_id: String },
    #[serde(rename = "thread.token_usage_updated")]
    ThreadTokenUsageUpdated { token_usage: CodexTokenUsageRaw },
    #[serde(rename = "item.completed")]
    ItemCompleted { item: CodexItem },
    #[serde(other)]
    Other,
}

impl CodexEvent {
    /// Translate this event into the wrapper-agnostic [`WorkerEvent`] shape
    /// the headless dispatcher forwards to the server. Only assistant
    /// message text currently maps; other event types are dropped here (the
    /// server-side `worker_event_to_session_event` would discard them
    /// anyway).
    fn to_worker_event(&self) -> Option<WorkerEvent> {
        match self {
            CodexEvent::ItemCompleted {
                item: CodexItem::AgentMessage { text },
            } => Some(WorkerEvent::AssistantText { text: text.clone() }),
            _ => None,
        }
    }
}

#[derive(Deserialize, Default)]
struct CodexTokenUsageRaw {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

#[derive(Deserialize)]
#[serde(tag = "item_type")]
enum CodexItem {
    #[serde(rename = "AgentMessageItem")]
    AgentMessage { text: String },
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_config_to_codex_toml_basic() {
        let json = r#"{
            "mcpServers": {
                "my-server": {
                    "command": "npx",
                    "args": ["-y", "some-server"],
                    "env": {"API_KEY": "secret123"}
                }
            }
        }"#;

        let toml_str = mcp_config_to_codex_toml(json).unwrap();
        let parsed: toml::map::Map<String, toml::Value> = toml::from_str(&toml_str).unwrap();

        let servers = parsed["mcp_servers"].as_table().unwrap();
        let server = servers["my-server"].as_table().unwrap();
        assert_eq!(server["command"].as_str().unwrap(), "npx");
        assert_eq!(
            server["args"].as_array().unwrap(),
            &[
                toml::Value::String("-y".to_string()),
                toml::Value::String("some-server".to_string()),
            ]
        );
        assert_eq!(
            server["env"].as_table().unwrap()["API_KEY"]
                .as_str()
                .unwrap(),
            "secret123"
        );
    }

    #[test]
    fn test_mcp_config_to_codex_toml_multiple_servers() {
        let json = r#"{
            "mcpServers": {
                "server-a": {"command": "cmd-a"},
                "server-b": {"command": "cmd-b", "args": ["--flag"]}
            }
        }"#;

        let toml_str = mcp_config_to_codex_toml(json).unwrap();
        let parsed: toml::map::Map<String, toml::Value> = toml::from_str(&toml_str).unwrap();

        let servers = parsed["mcp_servers"].as_table().unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers["server-a"]["command"].as_str().unwrap(), "cmd-a");
        assert_eq!(servers["server-b"]["command"].as_str().unwrap(), "cmd-b");
    }

    #[test]
    fn test_mcp_config_to_codex_toml_missing_mcp_servers() {
        let json = r#"{"other": "data"}"#;
        let result = mcp_config_to_codex_toml(json);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("mcpServers"));
    }

    #[test]
    fn test_mcp_config_to_codex_toml_no_optional_fields() {
        let json = r#"{"mcpServers": {"minimal": {"command": "run"}}}"#;
        let toml_str = mcp_config_to_codex_toml(json).unwrap();
        let parsed: toml::map::Map<String, toml::Value> = toml::from_str(&toml_str).unwrap();

        let server = parsed["mcp_servers"]["minimal"].as_table().unwrap();
        assert_eq!(server["command"].as_str().unwrap(), "run");
        assert!(!server.contains_key("args"));
        assert!(!server.contains_key("env"));
    }

    #[test]
    fn codex_event_token_usage_sets_usage() {
        let line = r#"{"type":"thread.token_usage_updated","token_usage":{"input_tokens":42,"output_tokens":13,"total_tokens":55}}"#;
        let event: CodexEvent = serde_json::from_str(line).unwrap();
        let mut state = CodexParseState::default();
        state.apply(event);
        assert_eq!(state.usage.input_tokens, 42);
        assert_eq!(state.usage.output_tokens, 13);
        assert_eq!(state.usage.cache_read_input_tokens, 0);
        assert_eq!(state.usage.cache_creation_input_tokens, 0);
    }

    #[test]
    fn codex_event_thread_started_captures_session_id() {
        let line = r#"{"type":"thread.started","thread_id":"abc-123"}"#;
        let event: CodexEvent = serde_json::from_str(line).unwrap();
        let mut state = CodexParseState::default();
        state.apply(event);
        assert_eq!(state.session_id.as_deref(), Some("abc-123"));
    }

    #[test]
    fn codex_event_item_completed_with_agent_message_sets_last_message() {
        let line = r#"{"type":"item.completed","item":{"item_type":"AgentMessageItem","text":"hello world"}}"#;
        let event: CodexEvent = serde_json::from_str(line).unwrap();
        let mut state = CodexParseState::default();
        state.apply(event);
        assert_eq!(state.last_message.as_deref(), Some("hello world"));
    }

    #[test]
    fn codex_event_unknown_variant_does_not_crash() {
        for line in [
            r#"{"type":"turn.started"}"#,
            r#"{"type":"item.started","item":{"item_type":"Whatever"}}"#,
            r#"{"type":"some_future_event","details":{"x":1}}"#,
        ] {
            let event: CodexEvent =
                serde_json::from_str(line).expect("should fall through to Other");
            let mut state = CodexParseState::default();
            state.apply(event);
            assert_eq!(state.usage, TokenUsage::default());
            assert!(state.session_id.is_none());
            assert!(state.last_message.is_none());
        }
    }

    #[test]
    fn codex_event_token_usage_overwrites_on_each_event() {
        let mut state = CodexParseState::default();
        let line1 = r#"{"type":"thread.token_usage_updated","token_usage":{"input_tokens":10,"output_tokens":5}}"#;
        let line2 = r#"{"type":"thread.token_usage_updated","token_usage":{"input_tokens":40,"output_tokens":12}}"#;
        state.apply(serde_json::from_str::<CodexEvent>(line1).unwrap());
        state.apply(serde_json::from_str::<CodexEvent>(line2).unwrap());
        assert_eq!(state.usage.input_tokens, 40);
        assert_eq!(state.usage.output_tokens, 12);
    }

    #[test]
    fn codex_item_with_non_agent_message_is_ignored() {
        let line = r#"{"type":"item.completed","item":{"item_type":"WebSearchItem","query":"x"}}"#;
        let event: CodexEvent = serde_json::from_str(line).unwrap();
        let mut state = CodexParseState::default();
        state.apply(event);
        assert!(state.last_message.is_none());
    }

    #[test]
    fn session_state_if_exists_returns_some_for_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("session.jsonl");
        std::fs::write(&path, b"line1\n").unwrap();
        let result = session_state_if_exists(path.clone(), SessionStateFormat::CodexJsonl);
        let r = result.expect("file exists → Some");
        assert_eq!(r.local_path, path);
        assert_eq!(r.format, SessionStateFormat::CodexJsonl);
    }

    #[tokio::test]
    async fn codex_new_errors_when_openai_api_key_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        let working_dir = tmp.path().to_path_buf();
        let env: HashMap<String, String> = HashMap::new();
        let result = Codex::new(None, working_dir, home, env, None).await;
        let err = match result {
            Ok(_) => panic!("expected Err"),
            Err(e) => e,
        };
        let err_str = err.to_string();
        assert!(
            err_str.contains(ENV_OPENAI_API_KEY),
            "error should mention {ENV_OPENAI_API_KEY}; got: {err_str}"
        );
    }

    #[test]
    fn to_worker_event_maps_agent_message_to_assistant_text() {
        let event = CodexEvent::ItemCompleted {
            item: CodexItem::AgentMessage {
                text: "hello".to_string(),
            },
        };
        match event.to_worker_event() {
            Some(WorkerEvent::AssistantText { text }) => assert_eq!(text, "hello"),
            other => panic!("expected AssistantText, got {other:?}"),
        }
    }

    #[test]
    fn to_worker_event_drops_non_assistant_events() {
        for event in [
            CodexEvent::ThreadStarted {
                thread_id: "tid".to_string(),
            },
            CodexEvent::ThreadTokenUsageUpdated {
                token_usage: CodexTokenUsageRaw::default(),
            },
            CodexEvent::ItemCompleted {
                item: CodexItem::Other,
            },
            CodexEvent::Other,
        ] {
            assert!(event.to_worker_event().is_none());
        }
    }

    /// `try_materialize` on Codex is a stub today — every input shape (well
    /// formed Claude payload, garbage bytes, empty) must surface
    /// `NotImplemented` so the dispatcher uniformly falls back to transcript
    /// replay. Constructed via `Codex { ... }` directly so the test doesn't
    /// need to run `codex login`.
    #[test]
    fn codex_try_materialize_always_returns_not_implemented() {
        let tmp = tempfile::tempdir().unwrap();
        let codex = Codex {
            model: None,
            working_dir: tmp.path().to_path_buf(),
            env: HashMap::new(),
            _config_guard: None,
            output_dir: tempfile::Builder::new()
                .prefix("codex-session-test")
                .tempdir()
                .unwrap(),
            bypass_sandbox: true,
        };

        for input in [
            b"".as_ref(),
            b"not json".as_ref(),
            b"{\"version\":\"v1\",\"session_id\":\"x\"}".as_ref(),
        ] {
            let result = codex.try_materialize(input);
            assert!(
                matches!(result, Err(MaterializeError::NotImplemented)),
                "expected NotImplemented for input {input:?}, got {result:?}"
            );
        }
    }
}
