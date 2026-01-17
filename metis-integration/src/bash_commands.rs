use std::{collections::HashMap, path::Path};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use metis::{
    cli, client::MetisClientInterface, command::worker_run::WorkerCommands, config::AppConfig,
};

pub struct BashCommands {
    pub commands: Vec<String>,
    pub client: Box<dyn MetisClientInterface>,
    pub app_config: AppConfig,
}

impl BashCommands {
    fn parse_command_tokens(command: &str) -> Vec<String> {
        // Simple tokenization: split by whitespace
        // This will work for commands like "metis patches create ..."
        // Note: this won't handle quoted strings perfectly, but for the use case
        // where we're checking if the first token is "metis", it should be sufficient
        command.split_whitespace().map(|s| s.to_string()).collect()
    }

    async fn run_custom_command(
        &self,
        command: &str,
        working_dir: &Path,
        env: &HashMap<String, String>,
    ) -> Result<String> {
        let tokens = Self::parse_command_tokens(command);
        let first_token = tokens.first().map(|s| s.as_str());

        // If the first token is "metis", use cli::run_with_client_and_config
        if first_token == Some("metis") {
            // Skip the first token ("metis") and use the rest as args
            let args: Vec<String> = tokens.into_iter().skip(1).collect();
            // Note: This requires the future to be Send, but cli::run_with_client_and_config
            // may use stdout locks which are not Send. In practice, this works in test contexts
            // where the trait is used directly (not spawned across threads).
            cli::run_with_client_and_config(args, self.client.as_ref(), &self.app_config)
                .await
                .context("failed to run metis command via cli")?;
            // Return empty string as metis commands don't produce output to capture
            return Ok(String::new());
        }

        // Otherwise, run as a shell command
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(working_dir)
            .envs(env)
            .output()
            .await
            .context("failed to spawn custom run command")?;

        if !output.status.success() {
            bail!(
                "custom run command '{command}' failed with status {status}",
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
        for command in &self.commands {
            last_output = self
                .run_custom_command(command, working_dir, env)
                .await
                .with_context(|| format!("failed to run command '{command}'"))?;
        }
        Ok(last_output)
    }
}
