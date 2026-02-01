use std::{
    collections::HashMap,
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use metis_common::constants::{
    ENV_ANTHROPIC_API_KEY, ENV_CLAUDE_CODE_OAUTH_TOKEN, ENV_OPENAI_API_KEY,
};
use serde_json::Value;
use tokio::{
    fs,
    io::{self, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

#[async_trait]
pub trait WorkerCommands: Send + Sync {
    async fn run(
        &self,
        prompt: &str,
        model: Option<&str>,
        openai_api_key: Option<String>,
        anthropic_api_key: Option<String>,
        claude_code_oauth_token: Option<String>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
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

const MAX_LINE_LEN: usize = 160;
const TOOL_SNIPPET_LEN: usize = 400;
const TEXT_BLOCK_GAP: &str = "\n";

struct PendingToolCall {
    name: String,
    summary: String,
    started_at: Instant,
}

impl PendingToolCall {
    fn new(name: String, summary: String) -> Self {
        Self {
            name,
            summary,
            started_at: Instant::now(),
        }
    }
}

struct StreamFormatter {
    pending_tools: HashMap<String, PendingToolCall>,
}

impl StreamFormatter {
    fn new() -> Self {
        Self {
            pending_tools: HashMap::new(),
        }
    }

    fn handle_line(&mut self, raw_line: &str) -> Vec<String> {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            return vec![];
        }

        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            return vec![];
        };

        match value.get("type").and_then(Value::as_str) {
            Some("assistant") => self.handle_assistant(&value),
            Some("user") => self.handle_user(&value),
            _ => vec![],
        }
    }

    fn handle_assistant(&mut self, value: &Value) -> Vec<String> {
        let mut renders = Vec::new();
        let Some(content) = value
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
        else {
            return renders;
        };

        for chunk in content {
            let Some(chunk_type) = chunk.get("type").and_then(Value::as_str) else {
                continue;
            };
            match chunk_type {
                "text" => {
                    if let Some(text) = chunk.get("text").and_then(Value::as_str) {
                        renders.push(format_block("assistant>", text));
                    }
                }
                "thinking" | "reasoning" => {
                    if let Some(text) = chunk.get("text").and_then(Value::as_str) {
                        renders.push(format_block("reasoning>", text));
                    }
                }
                "tool_use" => {
                    if let Some(rendered) = self.handle_tool_use(chunk) {
                        renders.push(rendered);
                    }
                }
                _ => {}
            }
        }

        renders
    }

    fn handle_tool_use(&mut self, chunk: &Value) -> Option<String> {
        let id = chunk.get("id").and_then(Value::as_str)?.to_owned();
        let name = chunk
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("tool")
            .to_owned();
        let summary = summarize_input(chunk.get("input"));
        self.pending_tools
            .insert(id, PendingToolCall::new(name.clone(), summary.clone()));

        Some(format!("tool> {name} - {summary}{TEXT_BLOCK_GAP}"))
    }

    fn handle_user(&mut self, value: &Value) -> Vec<String> {
        let mut renders = Vec::new();
        let Some(content) = value
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
        else {
            return renders;
        };

        for chunk in content {
            let Some("tool_result") = chunk.get("type").and_then(Value::as_str) else {
                continue;
            };
            let Some(tool_id) = chunk.get("tool_use_id").and_then(Value::as_str) else {
                continue;
            };
            let result_text = extract_tool_result_text(chunk.get("content"));
            let is_error = chunk
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if let Some(pending) = self.pending_tools.remove(tool_id) {
                let duration = pending.started_at.elapsed();
                renders.push(format_tool_result(
                    &pending.name,
                    &pending.summary,
                    duration,
                    &result_text,
                    is_error,
                ));
            }
        }

        renders
    }
}

fn format_block(prefix: &str, text: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }

    let mut rendered = String::new();
    for line in text.replace('\r', "").lines() {
        if line.trim().is_empty() {
            rendered.push('\n');
            continue;
        }
        rendered.push_str(prefix);
        rendered.push(' ');
        rendered.push_str(&truncate(line, MAX_LINE_LEN));
        rendered.push('\n');
    }
    rendered.push_str(TEXT_BLOCK_GAP);
    rendered
}

fn summarize_input(value: Option<&Value>) -> String {
    let summary = match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Object(map)) => map
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                map.get("command")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| serde_json::to_string(map).unwrap_or_else(|_| "input".to_string())),
        Some(other) => serde_json::to_string(other).unwrap_or_else(|_| String::new()),
        None => String::new(),
    };

    if summary.is_empty() {
        "started".to_string()
    } else {
        truncate(&summary, MAX_LINE_LEN)
    }
}

fn extract_tool_result_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => truncate(text, TOOL_SNIPPET_LEN),
        Some(Value::Array(items)) => truncate(
            &items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
                .join("\n"),
            TOOL_SNIPPET_LEN,
        ),
        Some(other) => truncate(
            &serde_json::to_string(other).unwrap_or_else(|_| String::new()),
            TOOL_SNIPPET_LEN,
        ),
        None => String::new(),
    }
}

fn format_tool_result(
    name: &str,
    summary: &str,
    duration: Duration,
    content: &str,
    is_error: bool,
) -> String {
    let mut rendered = String::new();
    let status = if is_error {
        "tool error>"
    } else {
        "tool done>"
    };
    rendered.push_str(&format!(
        "{status} {name} - {summary} ({:.1}s)\n",
        duration.as_secs_f32()
    ));
    if !content.trim().is_empty() {
        for line in content.lines() {
            rendered.push_str("  ");
            rendered.push_str(&truncate(line, MAX_LINE_LEN));
            rendered.push('\n');
        }
    }
    rendered.push_str(TEXT_BLOCK_GAP);
    rendered
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    text.chars()
        .take(limit.saturating_sub(3))
        .collect::<String>()
        + "..."
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
        openai_api_key: Option<String>,
        _anthropic_api_key: Option<String>,
        _claude_code_oauth_token: Option<String>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
    ) -> Result<String> {
        self.login(openai_api_key.as_deref()).await?;
        Self::run_codex(prompt, model, working_dir, env, output_path)
            .await
            .with_context(|| "failed to execute codex for worker context")
    }
}

impl ClaudeCommands {
    async fn run_claude(
        prompt: &str,
        model: Option<&str>,
        anthropic_api_key: Option<String>,
        claude_code_oauth_token: Option<String>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
    ) -> Result<String> {
        if let Some(dir) = output_path.parent() {
            fs::create_dir_all(dir)
                .await
                .with_context(|| format!("failed to create claude output directory {dir:?}"))?;
        }

        let anthropic_api_key = anthropic_api_key.filter(|value| !value.trim().is_empty());
        let claude_code_oauth_token =
            claude_code_oauth_token.filter(|value| !value.trim().is_empty());

        if anthropic_api_key.is_none() && claude_code_oauth_token.is_none() {
            return Err(anyhow!(
                "Either {ENV_CLAUDE_CODE_OAUTH_TOKEN} or {ENV_ANTHROPIC_API_KEY} must be provided via CLI flags or environment"
            ));
        }

        let mut command = Command::new("claude");
        command.arg("--print");
        command.arg("--dangerously-skip-permissions");
        command.arg("--verbose");
        command.arg("--output-format");
        command.arg("stream-json");
        if let Some(model) = model {
            command.arg("--model");
            command.arg(model);
        }
        command.current_dir(working_dir).envs(env);
        if let Some(key) = anthropic_api_key.as_ref() {
            command.env(ENV_ANTHROPIC_API_KEY, key);
        }
        if let Some(token) = claude_code_oauth_token.as_ref() {
            command.env(ENV_CLAUDE_CODE_OAUTH_TOKEN, token);
        }

        command.arg(prompt);

        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to spawn claude command")?;

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

        let status = child
            .wait()
            .await
            .context("failed waiting for claude command to finish")?;
        let stderr_buf = stderr_handle
            .await
            .context("failed to join claude stderr task")??;

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

        Ok(stdout_buf)
    }
}

#[async_trait]
impl WorkerCommands for ClaudeCommands {
    async fn run(
        &self,
        prompt: &str,
        model: Option<&str>,
        _openai_api_key: Option<String>,
        anthropic_api_key: Option<String>,
        claude_code_oauth_token: Option<String>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
    ) -> Result<String> {
        Self::run_claude(
            prompt,
            model,
            anthropic_api_key,
            claude_code_oauth_token,
            working_dir,
            env,
            output_path,
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
        openai_api_key: Option<String>,
        anthropic_api_key: Option<String>,
        claude_code_oauth_token: Option<String>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
    ) -> Result<String> {
        match model.filter(|value| is_claude_model(value)) {
            Some(_) => {
                self.claude
                    .run(
                        prompt,
                        model,
                        openai_api_key,
                        anthropic_api_key,
                        claude_code_oauth_token.clone(),
                        working_dir,
                        env,
                        output_path,
                    )
                    .await
            }
            None => {
                self.codex
                    .run(
                        prompt,
                        model,
                        openai_api_key,
                        anthropic_api_key,
                        claude_code_oauth_token,
                        working_dir,
                        env,
                        output_path,
                    )
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_input_prefers_description_then_command() {
        let mut map = serde_json::Map::new();
        map.insert("description".to_string(), Value::String("describe".into()));
        map.insert("command".to_string(), Value::String("cmd".into()));
        assert_eq!(
            summarize_input(Some(&Value::Object(map.clone()))),
            "describe"
        );

        map.remove("description");
        assert_eq!(
            summarize_input(Some(&Value::Object(map))),
            "cmd".to_string()
        );
    }

    #[test]
    fn format_block_prefixes_each_line() {
        let rendered = format_block("assistant>", "hello\nworld");
        assert!(rendered.contains("assistant> hello"));
        assert!(rendered.contains("assistant> world"));
    }

    #[test]
    fn extract_tool_result_handles_string_and_array() {
        let string_value = Value::String("first line\nsecond".into());
        assert!(extract_tool_result_text(Some(&string_value)).contains("first line"));

        let array_value = Value::Array(vec![
            Value::String("alpha".into()),
            Value::String("beta".into()),
        ]);
        let rendered = extract_tool_result_text(Some(&array_value));
        assert!(rendered.contains("alpha"));
        assert!(rendered.contains("beta"));
    }

    #[test]
    fn format_tool_result_includes_duration_and_status() {
        let rendered = format_tool_result(
            "Bash",
            "run command",
            Duration::from_millis(1500),
            "ok",
            false,
        );
        assert!(rendered.contains("tool done> Bash - run command (1.5s)"));
        assert!(rendered.contains("ok"));
    }

    #[test]
    fn truncate_adds_ellipsis_when_needed() {
        let result = truncate("abcdef", 5);
        assert_eq!(result, "ab...");
    }
}
