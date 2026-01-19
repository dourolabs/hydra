use std::{collections::HashMap, path::Path};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use metis::worker_commands::WorkerCommands;

use super::test_helpers::metis_bin;

pub struct BashCommands {
    pub commands: Vec<String>,
}

impl BashCommands {
    async fn run_custom_command(
        &self,
        command_string: &str,
        working_dir: &Path,
        env: &HashMap<String, String>,
    ) -> Result<String> {
        let first_token = command_string.split_whitespace().next();
        let command_to_run = if first_token == Some("metis") {
            let metis_path = metis_bin();
            command_string.replacen("metis", &metis_path.to_string_lossy(), 1)
        } else {
            command_string.to_string()
        };

        let output = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(&command_to_run)
            .current_dir(working_dir)
            .envs(env)
            .output()
            .await
            .context("failed to spawn custom run command")?;

        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "custom run command '{command_to_run}' failed with status {status}\nstdout:\n{stdout}\nstderr:\n{stderr}",
                status = output.status,
                stdout = stdout,
                stderr = stderr
            );
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

#[async_trait]
impl WorkerCommands for BashCommands {
    async fn run(
        &self,
        _prompt: &str,
        _openai_api_key: Option<String>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        _output_path: &Path,
    ) -> Result<String> {
        let mut last_output = String::new();
        for command_string in &self.commands {
            last_output = self
                .run_custom_command(command_string, working_dir, env)
                .await
                .with_context(|| format!("failed to run command '{command_string}'"))?;
        }
        Ok(last_output)
    }
}
