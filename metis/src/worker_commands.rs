use std::{collections::HashMap, path::Path, process::Stdio};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use metis_common::constants::{ENV_ANTHROPIC_API_KEY, ENV_OPENAI_API_KEY};
use tokio::{fs, io::AsyncWriteExt, process::Command};

#[async_trait]
pub trait WorkerCommands: Send + Sync {
    async fn run(
        &self,
        prompt: &str,
        model: Option<&str>,
        openai_api_key: Option<String>,
        anthropic_api_key: Option<String>,
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
    model.to_ascii_lowercase().contains("claude")
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
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
    ) -> Result<String> {
        if let Some(dir) = output_path.parent() {
            fs::create_dir_all(dir)
                .await
                .with_context(|| format!("failed to create claude output directory {dir:?}"))?;
        }

        let anthropic_api_key = anthropic_api_key
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "{ENV_ANTHROPIC_API_KEY} must be provided via --anthropic-api-key or environment"
                )
            })?;

        let mut command = Command::new("claude");
        command.arg("--print");
        if let Some(model) = model {
            command.arg("--model");
            command.arg(model);
        }
        let output = command
            .arg(prompt)
            .current_dir(working_dir)
            .envs(env)
            .env(ENV_ANTHROPIC_API_KEY, anthropic_api_key)
            .output()
            .await
            .context("failed to spawn claude command")?;

        if !output.status.success() {
            return Err(anyhow!(
                "claude command failed with status {}",
                output.status
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        fs::write(output_path, &stdout)
            .await
            .with_context(|| format!("failed to write claude output to {output_path:?}"))?;

        Ok(stdout)
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
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
    ) -> Result<String> {
        Self::run_claude(
            prompt,
            model,
            anthropic_api_key,
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
                        working_dir,
                        env,
                        output_path,
                    )
                    .await
            }
        }
    }
}
