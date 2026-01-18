use std::{
    collections::HashMap,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use metis_common::constants::ENV_OPENAI_API_KEY;

use crate::exec::run_codex;

#[async_trait]
pub trait WorkerCommands: Send + Sync {
    async fn run(
        &self,
        prompt: &str,
        openai_api_key: Option<String>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
    ) -> Result<String>;
}

pub struct CodexCommands {}
impl CodexCommands {
    fn login(&self, openai_api_key: Option<&str>) -> Result<()> {
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
                .with_context(|| format!("failed to write {ENV_OPENAI_API_KEY} to codex login"))?;
        }

        let status = login_cmd
            .wait()
            .context("failed waiting for codex login to finish")?;
        if !status.success() {
            return Err(anyhow!("codex login failed with status {status}"));
        }

        Ok(())
    }
}

#[async_trait]
impl WorkerCommands for CodexCommands {
    async fn run(
        &self,
        prompt: &str,
        openai_api_key: Option<String>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
    ) -> Result<String> {
        self.login(openai_api_key.as_deref())?;
        run_codex(prompt, working_dir, env, output_path)
            .await
            .with_context(|| "failed to execute codex for worker context")
    }
}
