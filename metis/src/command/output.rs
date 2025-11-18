use crate::{client::MetisClient, config::AppConfig};
use anyhow::{bail, Context, Result};
use std::process::Command;

pub async fn run(config: &AppConfig, job: String, apply: bool) -> Result<()> {
    let job_id = job.trim();
    if job_id.is_empty() {
        bail!("Job ID must not be empty.");
    }
    let job_id = job_id.to_string();
    let client = MetisClient::from_config(config)?;

    println!("Fetching output for job '{}' via metis-server…", job_id);

    let response = client.get_job_output(&job_id).await?;
    
    if apply {
        println!("\nApplying patch to current git repository…");
        
        // Check if we're in a git repository
        let git_check = Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .output()
            .context("Failed to check if current directory is a git repository. Is git installed?")?;
        
        if !git_check.status.success() {
            bail!("Current directory is not a git repository. Cannot apply patch.");
        }

        // Apply the patch using git apply
        let mut apply_cmd = Command::new("git");
        apply_cmd.arg("apply");
        
        let mut apply_result = apply_cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("Failed to spawn git apply command")?;

        let patch_bytes = response.output.patch.as_bytes();
        {
            let mut stdin = apply_result.stdin.take().ok_or_else(|| anyhow::anyhow!("Failed to open stdin for git apply"))?;
            use std::io::Write;
            stdin.write_all(patch_bytes).context("Failed to write patch to git apply")?;
        }

        let output = apply_result
            .wait_with_output()
            .context("Failed to wait for git apply to complete")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to apply patch: {}", stderr);
        }

        println!("Patch applied successfully.");
    } else {
        println!("\nLast agent message:\n{}\n", response.output.last_message);
        println!("Patch:\n{}", response.output.patch);
    }

    Ok(())
}
