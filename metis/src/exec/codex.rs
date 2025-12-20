use std::path::Path;

use anyhow::{anyhow, Context, Result};
use tokio::{fs, process::Command};

use crate::constants;

use super::AsyncOp;

pub(super) fn codex(prompt: String, continuation: rhai::FnPtr) -> (AsyncOp, rhai::FnPtr) {
    (AsyncOp::Codex { prompt }, continuation)
}

pub(super) async fn evaluate_codex_op(prompt: &str) -> Result<String> {
    let output_path = Path::new(constants::METIS_DIR)
        .join(constants::OUTPUT_DIR)
        .join(constants::OUTPUT_TXT_FILE);
    if let Some(dir) = output_path.parent() {
        fs::create_dir_all(dir)
            .await
            .with_context(|| format!("failed to create codex output directory {dir:?}"))?;
    }

    let status = Command::new("codex")
        .args([
            "exec",
            "-o",
            output_path
                .to_str()
                .expect("codex output path should be valid UTF-8"),
            "--dangerously-bypass-approvals-and-sandbox",
            prompt,
        ])
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
