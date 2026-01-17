use std::{collections::HashMap, path::Path};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use metis::{
    cli, client::MetisClientInterface, command::worker_run::WorkerCommands, config::AppConfig,
};

pub struct BashCommands {
    pub commands: Vec<Vec<String>>,
    pub client: Box<dyn MetisClientInterface>,
    pub app_config: AppConfig,
}

impl BashCommands {
    async fn run_custom_command(
        &self,
        tokens: &[String],
        working_dir: &Path,
        env: &HashMap<String, String>,
    ) -> Result<String> {
        let first_token = tokens.first().map(|s| s.as_str());

        // If the first token is "metis", use cli::run_with_client_and_config
        if first_token == Some("metis") {
            // Skip the first token ("metis") and use the rest as args
            let args: Vec<String> = tokens.iter().skip(1).cloned().collect();

            cli::run_with_client_and_config(args, self.client.as_ref(), &self.app_config)
                .await
                .context("failed to run metis command via cli")?;
            // Return empty string as metis commands don't produce output to capture
            return Ok(String::new());
        }

        // Otherwise, run as a shell command by reconstructing the command from tokens
        // Join tokens with spaces to reconstruct the command string
        let command_string = tokens.join(" ");
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&command_string)
            .current_dir(working_dir)
            .envs(env)
            .output()
            .await
            .context("failed to spawn custom run command")?;

        if !output.status.success() {
            bail!(
                "custom run command '{command_string}' failed with status {status}",
                status = output.status
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
        for tokens in &self.commands {
            let command_str = tokens.join(" ");
            last_output = self
                .run_custom_command(tokens, working_dir, env)
                .await
                .with_context(|| format!("failed to run command '{command_str}'"))?;
        }
        Ok(last_output)
    }
}
