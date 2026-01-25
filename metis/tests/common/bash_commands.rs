use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use metis::worker_commands::WorkerCommands;
use metis_common::constants::ENV_METIS_ISSUE_ID;

use super::test_helpers::metis_bin;

#[derive(Clone, Debug)]
pub struct CommandOutput {
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub status: i32,
}

pub struct BashCommands {
    pub commands: Vec<String>,
    outputs: Arc<Mutex<Vec<CommandOutput>>>,
    fail_after_run: bool,
}

impl BashCommands {
    pub fn new_with_failure(commands: Vec<String>, fail_after_run: bool) -> Self {
        Self {
            commands,
            outputs: Arc::new(Mutex::new(Vec::new())),
            fail_after_run,
        }
    }

    pub fn outputs(&self) -> Vec<CommandOutput> {
        self.outputs
            .lock()
            .expect("failed to lock command outputs")
            .clone()
    }

    async fn run_custom_command(
        &self,
        command_string: &str,
        working_dir: &Path,
        env: &HashMap<String, String>,
    ) -> Result<CommandOutput> {
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
            .env_remove(ENV_METIS_ISSUE_ID)
            .envs(env)
            .output()
            .await
            .context("failed to spawn custom run command")?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let status_code = output.status.code().unwrap_or(-1);
        let command_output = CommandOutput {
            command: command_to_run.clone(),
            stdout: stdout.clone(),
            stderr: stderr.clone(),
            status: status_code,
        };
        self.outputs
            .lock()
            .expect("failed to store command outputs")
            .push(command_output.clone());

        if !output.status.success() {
            bail!(
                "custom run command '{command_to_run}' failed with status {status}\nstdout:\n{stdout}\nstderr:\n{stderr}",
                status = output.status,
                stdout = stdout,
                stderr = stderr
            );
        }

        Ok(command_output)
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
            let output = self
                .run_custom_command(command_string, working_dir, env)
                .await
                .with_context(|| format!("failed to run command '{command_string}'"))?;
            last_output = output.stdout.clone();
        }

        if self.fail_after_run {
            bail!("BashCommands configured to fail after running commands");
        }

        Ok(last_output)
    }
}
