use std::{collections::HashMap, path::Path, process::Stdio};

use crate::claude_formatter::StreamFormatter;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use metis_common::constants::{
    ENV_ANTHROPIC_API_KEY, ENV_CLAUDE_CODE_OAUTH_TOKEN, ENV_OPENAI_API_KEY,
};
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
