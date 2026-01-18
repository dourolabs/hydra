use std::{collections::HashMap, path::Path};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use brush_parser::{tokenize_str, Token};
use metis::{
    cli, client::MetisClientInterface, command::worker_run::WorkerCommands, config::AppConfig,
};

pub struct BashCommands {
    pub commands: Vec<String>,
    pub client: Box<dyn MetisClientInterface>,
    pub app_config: AppConfig,
}

impl BashCommands {
    async fn run_custom_command(
        &self,
        command_string: &str,
        working_dir: &Path,
        env: &HashMap<String, String>,
    ) -> Result<String> {
        // Parse command to check if it starts with "metis"
        let tokens = tokenize_str(command_string).context("failed to tokenize command")?;
        let mut words = Vec::new();
        for token in tokens {
            if let Token::Word(word, _) = token {
                words.push(word.to_string());
            }
        }

        // If the first token is "metis", use cli::run_with_client_and_config
        if words.first().map(|s| s.as_str()) == Some("metis") {
            cli::run_with_client_and_config(words, self.client.as_ref(), &self.app_config)
                .await
                .context("failed to run metis command via cli")?;
            // Return empty string as metis commands don't produce output to capture
            return Ok(String::new());
        }

        // Otherwise, run as a shell command using bash (preserves redirects like >>)
        let output = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(command_string)
            .current_dir(working_dir)
            .envs(env)
            .output()
            .await
            .context("failed to spawn custom run command")?;

        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "custom run command '{command_string}' failed with status {status}\nstdout:\n{stdout}\nstderr:\n{stderr}",
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
