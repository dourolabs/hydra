use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use tokio::process::Command;

use super::AsyncOp;

pub(super) fn shell(command: String, continuation: rhai::FnPtr) -> (AsyncOp, rhai::FnPtr) {
    (AsyncOp::Shell { command }, continuation)
}

pub(super) async fn evaluate_shell_command(
    command: &str,
    env: &HashMap<String, String>,
) -> Result<String> {
    let output = Command::new("bash")
        .args(["-c", command])
        .envs(env)
        .output()
        .await
        .with_context(|| format!("failed to spawn shell command: {command}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "shell command `{command}` failed with status {}{}",
            output.status,
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        ));
    }

    String::from_utf8(output.stdout)
        .map_err(|err| anyhow!("failed to decode shell command output as UTF-8: {err}"))
}
