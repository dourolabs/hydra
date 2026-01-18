use std::{collections::HashMap, path::Path};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use metis::{client::MetisClientInterface, command::worker_run::WorkerCommands, config::AppConfig};

use escargot::CargoBuild;

fn metis_bin() -> std::path::PathBuf {
    CargoBuild::new()
        .package("metis")     // workspace package name
        .bin("metis")         // binary target name
        .current_release()    // optional; or omit for debug build
        .run()
        .unwrap()
        .path()
        .to_path_buf()
}

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
        // Check if the first token (split on whitespace) is "metis"
        let first_token = command_string.split_whitespace().next();
        let command_to_run = if first_token == Some("metis") {
            let metis_path = metis_bin();
            // Simple string replacement: replace first occurrence of "metis" at word boundary
            // This works because we've already verified the first word is "metis"
            command_string.replacen("metis", &metis_path.to_string_lossy(), 1)
        } else {
            command_string.to_string()
        };

        // Run as a shell command using bash (preserves redirects like >>)
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
