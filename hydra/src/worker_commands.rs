use std::{collections::HashMap, path::Path, process::Stdio, time::Instant};

use crate::claude_formatter::StreamFormatter;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use hydra_common::constants::{
    ENV_ANTHROPIC_API_KEY, ENV_CLAUDE_CODE_OAUTH_TOKEN, ENV_OPENAI_API_KEY,
};
use serde_json::Value;
use tokio::{
    fs,
    io::{self, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

/// Grace period after the main process exits before killing the process group.
const PROCESS_GROUP_GRACE_PERIOD: std::time::Duration = std::time::Duration::from_secs(5);

/// Time to wait for SIGTERM to take effect before sending SIGKILL.
const SIGTERM_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Timeout for stdout/stderr pipe reads after process group kill.
/// If pipes don't EOF within this duration, we drop the handles and move on.
const PIPE_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Sends SIGTERM then SIGKILL to a process group.
///
/// The `pgid` is the process group ID (same as the leader's PID when spawned
/// with `process_group(0)`). Signals are sent via `kill(-pgid, sig)` so they
/// reach every process in the group.
#[cfg(unix)]
async fn kill_process_group(pgid: u32) {
    let neg_pgid = -(pgid as i32);

    // SIGTERM — give processes a chance to exit cleanly
    // SAFETY: kill with a negative pid signals the process group.
    unsafe {
        libc::kill(neg_pgid, libc::SIGTERM);
    }

    tokio::time::sleep(SIGTERM_WAIT).await;

    // SIGKILL — force-kill anything still alive
    unsafe {
        libc::kill(neg_pgid, libc::SIGKILL);
    }
}

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
    ) -> Result<String>;
}

pub struct CodexCommands;
pub struct ClaudeCommands;

pub struct ModelAwareCommands {
    codex: CodexCommands,
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

fn is_claude_model(model: &str) -> bool {
    let lc = model.to_ascii_lowercase();
    lc.contains("claude") || lc.contains("haiku") || lc.contains("sonnet") || lc.contains("opus")
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
fn mcp_config_to_codex_toml(mcp_json: &str) -> Result<String> {
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

/// Guard that cleans up ~/.codex/config.toml written for a Codex run.
struct CodexConfigGuard {
    config_path: std::path::PathBuf,
    codex_dir: std::path::PathBuf,
    created_dir: bool,
}

impl CodexConfigGuard {
    async fn cleanup(self) {
        let _ = fs::remove_file(&self.config_path).await;
        if self.created_dir {
            // Remove ~/.codex dir only if we created it and it's now empty
            let _ = fs::remove_dir(&self.codex_dir).await;
        }
    }
}

impl CodexCommands {
    async fn login(&self, openai_api_key: Option<&str>) -> Result<()> {
        let openai_api_key = openai_api_key.map(str::to_owned).ok_or_else(|| {
            anyhow!("{ENV_OPENAI_API_KEY} must be provided via --openai-api-key or environment")
        })?;

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

    /// Writes ~/.codex/config.toml with MCP server config and returns a guard
    /// that will clean it up when dropped. Uses the home directory (global
    /// Codex config) since hydra manages the agent's home directory.
    async fn write_codex_mcp_config(mcp_config: &str) -> Result<CodexConfigGuard> {
        let home = std::env::var("HOME").context("HOME environment variable not set")?;
        let codex_dir = std::path::PathBuf::from(home).join(".codex");
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

    async fn run_codex(
        prompt: &str,
        model: Option<&str>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
    ) -> Result<String> {
        if let Some(dir) = output_path.parent() {
            fs::create_dir_all(dir)
                .await
                .with_context(|| format!("failed to create codex output directory {dir:?}"))?;
        }

        let mut command = Command::new("codex");
        command
            .args([
                "exec",
                "--color",
                "always",
                "--skip-git-repo-check",
                "-o",
                output_path
                    .to_str()
                    .expect("codex output path should be valid UTF-8"),
                "--dangerously-bypass-approvals-and-sandbox",
            ])
            .current_dir(working_dir)
            .envs(env);
        #[cfg(unix)]
        command.process_group(0);
        if let Some(model) = model {
            command.arg("--model");
            command.arg(model);
        }
        command.arg(prompt);

        let status = command
            .status()
            .await
            .context("failed to spawn codex command")?;

        if !status.success() {
            return Err(anyhow!("codex command failed with status {status}"));
        }

        fs::read_to_string(output_path)
            .await
            .with_context(|| format!("failed to read codex output from {output_path:?}"))
    }
}

#[async_trait]
impl WorkerCommands for CodexCommands {
    async fn run(
        &self,
        prompt: &str,
        model: Option<&str>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
        mcp_config: Option<&str>,
    ) -> Result<String> {
        let openai_api_key = env.get(ENV_OPENAI_API_KEY).map(|s| s.as_str());
        self.login(openai_api_key).await?;

        // Write ~/.codex/config.toml if MCP config is provided
        let config_guard = if let Some(config_json) = mcp_config {
            Some(
                Self::write_codex_mcp_config(config_json)
                    .await
                    .context("failed to write Codex MCP config")?,
            )
        } else {
            None
        };

        let result = Self::run_codex(prompt, model, working_dir, env, output_path)
            .await
            .with_context(|| "failed to execute codex for worker context");

        // Clean up .codex/config.toml regardless of success or failure
        if let Some(guard) = config_guard {
            guard.cleanup().await;
        }

        result
    }
}

impl ClaudeCommands {
    /// Builds the argument list for the Claude CLI invocation.
    ///
    /// Uses `--` to separate options from the positional prompt argument.
    /// This is necessary because `--mcp-config` accepts variadic values
    /// (`<configs...>`), so without `--` it would consume the prompt text
    /// as an additional config path.
    fn build_claude_args(
        prompt: &str,
        model: Option<&str>,
        mcp_config_path: Option<&Path>,
    ) -> Vec<String> {
        let mut args = vec![
            "--print".to_string(),
            "--dangerously-skip-permissions".to_string(),
            "--verbose".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
        ];
        if let Some(model) = model {
            args.push("--model".to_string());
            args.push(model.to_string());
        }
        if let Some(mcp_path) = mcp_config_path {
            args.push("--mcp-config".to_string());
            args.push(mcp_path.to_string_lossy().to_string());
        }
        args.push("--".to_string());
        args.push(prompt.to_string());
        args
    }

    async fn run_claude(
        prompt: &str,
        model: Option<&str>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
        mcp_config_path: Option<&Path>,
    ) -> Result<String> {
        if let Some(dir) = output_path.parent() {
            fs::create_dir_all(dir)
                .await
                .with_context(|| format!("failed to create claude output directory {dir:?}"))?;
        }

        let has_anthropic_key = env
            .get(ENV_ANTHROPIC_API_KEY)
            .is_some_and(|v| !v.trim().is_empty());
        let has_oauth_token = env
            .get(ENV_CLAUDE_CODE_OAUTH_TOKEN)
            .is_some_and(|v| !v.trim().is_empty());

        if !has_anthropic_key && !has_oauth_token {
            return Err(anyhow!(
                "Either {ENV_CLAUDE_CODE_OAUTH_TOKEN} or {ENV_ANTHROPIC_API_KEY} must be provided in the job context environment"
            ));
        }

        let args = Self::build_claude_args(prompt, model, mcp_config_path);

        let mut command = Command::new("claude");
        command.args(&args);
        command.current_dir(working_dir).envs(env);
        #[cfg(unix)]
        command.process_group(0);

        eprintln!("Claude CLI args: {args:?}");

        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to spawn claude command")?;

        let spawn_time = Instant::now();
        let pid = child.id().unwrap_or(0);
        println!("Claude process spawned (PID: {pid})");

        #[cfg(unix)]
        let child_pgid = child.id();

        let child_stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("failed to capture stdout for claude command"))?;
        let child_stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("failed to capture stderr for claude command"))?;

        let stderr_handle = tokio::spawn(async move {
            let mut stderr_buf = Vec::new();
            tokio::io::BufReader::new(child_stderr)
                .read_to_end(&mut stderr_buf)
                .await
                .context("failed to read claude stderr")?;
            Ok::<Vec<u8>, anyhow::Error>(stderr_buf)
        });

        // Spawn stdout reading into a separate task so we can race it against
        // the child process exiting. This prevents hanging when a background
        // process inherits the stdout pipe fd.
        let mut stdout_handle = tokio::spawn(async move {
            let mut formatter = StreamFormatter::new();
            let mut reader = BufReader::new(child_stdout);
            let mut stdout_buf = String::new();
            let mut stdout_writer = io::stdout();
            let mut line = String::new();
            loop {
                line.clear();
                let read = reader
                    .read_line(&mut line)
                    .await
                    .context("failed to read claude stdout")?;
                if read == 0 {
                    let elapsed = spawn_time.elapsed().as_secs_f64();
                    println!("Claude stdout EOF reached (PID: {pid}, elapsed: {elapsed:.2}s)");
                    break;
                }
                for formatted in formatter.handle_line(&line) {
                    stdout_writer
                        .write_all(formatted.as_bytes())
                        .await
                        .context("failed to stream claude stdout")?;
                    stdout_writer
                        .flush()
                        .await
                        .context("failed to flush claude stdout")?;
                    stdout_buf.push_str(&formatted);
                }
            }
            let last_message = formatter.last_assistant_text().map(str::to_owned);
            Ok::<(String, Option<String>), anyhow::Error>((stdout_buf, last_message))
        });

        // Wait for the main claude process to exit.
        println!("Waiting for claude process to exit (PID: {pid})…");
        let status = child
            .wait()
            .await
            .context("failed waiting for claude command to finish")?;
        let wait_elapsed = spawn_time.elapsed().as_secs_f64();
        println!(
            "Claude process exited (PID: {pid}, status: {status}, elapsed: {wait_elapsed:.2}s)"
        );

        // The main process has exited. Give stdout a grace period to reach EOF
        // (it will if no background processes inherited the pipe).
        let (stdout_buf, last_message) =
            match tokio::time::timeout(PROCESS_GROUP_GRACE_PERIOD, &mut stdout_handle).await {
                Ok(join_result) => {
                    // stdout finished within the grace period — use the result directly.
                    join_result.context("failed to join claude stdout task")??
                }
                Err(_) => {
                    // Grace period expired; stdout is still open (orphaned subprocesses).
                    // Kill the process group to close inherited fds, then await with a
                    // longer timeout.
                    #[cfg(unix)]
                    if let Some(pgid) = child_pgid {
                        eprintln!(
                            "claude process exited but stdout pipe still open; \
                             killing process group {pgid}"
                        );
                        kill_process_group(pgid).await;
                    }

                    match tokio::time::timeout(PIPE_READ_TIMEOUT, stdout_handle).await {
                        Ok(join_result) => {
                            join_result.context("failed to join claude stdout task")??
                        }
                        Err(_) => {
                            let timeout = PIPE_READ_TIMEOUT;
                            eprintln!(
                                "stdout pipe read timed out after {timeout:?} — \
                                 dropping handle and proceeding with partial output"
                            );
                            (String::new(), None)
                        }
                    }
                }
            };

        // Await stderr with a timeout so we never hang indefinitely.
        let stderr_result = tokio::time::timeout(PIPE_READ_TIMEOUT, stderr_handle).await;
        let stderr_buf = match stderr_result {
            Ok(join_result) => join_result.context("failed to join claude stderr task")??,
            Err(_) => {
                let timeout = PIPE_READ_TIMEOUT;
                eprintln!(
                    "stderr pipe read timed out after {timeout:?} — \
                     dropping handle and proceeding without stderr"
                );
                Vec::new()
            }
        };

        if !status.success() {
            return Err(anyhow!(
                "claude command failed with status {}. stdout: {}. stderr: {}",
                status,
                stdout_buf,
                String::from_utf8_lossy(&stderr_buf)
            ));
        }

        fs::write(output_path, stdout_buf.as_bytes())
            .await
            .with_context(|| format!("failed to write claude output to {output_path:?}"))?;

        let last_message = last_message.unwrap_or(stdout_buf);
        Ok(last_message)
    }
}

#[async_trait]
impl WorkerCommands for ClaudeCommands {
    async fn run(
        &self,
        prompt: &str,
        model: Option<&str>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
        mcp_config: Option<&str>,
    ) -> Result<String> {
        // Write MCP config to a temp file if provided. The TempDir handle must
        // stay alive until run_claude completes so the file isn't cleaned up.
        let (_mcp_temp_dir, mcp_config_path) = if let Some(config_json) = mcp_config {
            let tmp_dir = tempfile::Builder::new()
                .prefix("mcp-config")
                .tempdir()
                .context("failed to create temporary directory for MCP config")?;
            let config_path = tmp_dir.path().join("mcp-config.json");
            fs::write(&config_path, config_json)
                .await
                .context("failed to write MCP config to temp file")?;
            (Some(tmp_dir), Some(config_path))
        } else {
            (None, None)
        };
        Self::run_claude(
            prompt,
            model,
            working_dir,
            env,
            output_path,
            mcp_config_path.as_deref(),
        )
        .await
        .with_context(|| "failed to execute claude for worker context")
    }
}

#[async_trait]
impl WorkerCommands for ModelAwareCommands {
    async fn run(
        &self,
        prompt: &str,
        model: Option<&str>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
        mcp_config: Option<&str>,
    ) -> Result<String> {
        match model.filter(|value| is_claude_model(value)) {
            Some(_) => {
                self.claude
                    .run(prompt, model, working_dir, env, output_path, mcp_config)
                    .await
            }
            None => {
                self.codex
                    .run(prompt, model, working_dir, env, output_path, mcp_config)
                    .await
            }
        }
    }
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
    fn test_build_claude_args_without_mcp_config() {
        let args = ClaudeCommands::build_claude_args("Do something", None, None);
        assert_eq!(
            args,
            vec![
                "--print",
                "--dangerously-skip-permissions",
                "--verbose",
                "--output-format",
                "stream-json",
                "--",
                "Do something",
            ]
        );
    }

    #[test]
    fn test_build_claude_args_with_mcp_config() {
        let mcp_path = Path::new("/tmp/mcp-config/mcp-config.json");
        let args = ClaudeCommands::build_claude_args(
            "Do something",
            Some("claude-sonnet-4-6"),
            Some(mcp_path),
        );
        assert_eq!(
            args,
            vec![
                "--print",
                "--dangerously-skip-permissions",
                "--verbose",
                "--output-format",
                "stream-json",
                "--model",
                "claude-sonnet-4-6",
                "--mcp-config",
                "/tmp/mcp-config/mcp-config.json",
                "--",
                "Do something",
            ]
        );
    }

    #[test]
    fn test_build_claude_args_prompt_after_separator() {
        // Verify the prompt always comes after "--" to prevent --mcp-config
        // from consuming it as a variadic argument.
        let mcp_path = Path::new("/tmp/config.json");
        let prompt = "You are a tester agent responsible for running tests...";
        let args = ClaudeCommands::build_claude_args(prompt, None, Some(mcp_path));

        let separator_pos = args.iter().position(|a| a == "--").unwrap();
        let prompt_pos = args.iter().position(|a| a == prompt).unwrap();
        assert!(
            prompt_pos == separator_pos + 1,
            "prompt must immediately follow '--' separator"
        );

        let mcp_config_pos = args.iter().position(|a| a == "--mcp-config").unwrap();
        assert!(
            mcp_config_pos < separator_pos,
            "--mcp-config must come before '--' separator"
        );
    }
}
