use std::{collections::HashMap, path::Path};

use anyhow::{anyhow, Context, Result};
use tokio::{fs, process::Command};

pub async fn run_codex(
    prompt: &str,
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
            prompt,
        ])
        .current_dir(working_dir)
        .envs(env);

    let status = command
        .status()
        .await
        .context("failed to spawn codex command")?;

    if !status.success() {
        return Err(anyhow!("codex command failed with status {status}"));
    }

    fs::read_to_string(&output_path)
        .await
        .with_context(|| format!("failed to read codex output from {output_path:?}"))
}
