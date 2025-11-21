use crate::{client::MetisClient, config::AppConfig};
use anyhow::{bail, Context, Result};
use std::{fs, path::PathBuf, process::Command};
use tempfile::NamedTempFile;

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
        
        // Find the git repository root
        let git_root_output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .context("Failed to find git repository root. Is git installed?")?;
        
        if !git_root_output.status.success() {
            bail!("Current directory is not a git repository. Cannot apply patch.");
        }
        
        let git_root = String::from_utf8(git_root_output.stdout)
            .context("Failed to parse git repository root")?
            .trim()
            .to_string();
        let git_root_path = PathBuf::from(&git_root);

        // Check if patch is empty
        let patch = response.output.patch;
        if patch.is_empty() {
            bail!("Patch is empty. Nothing to apply.");
        }
        println!("{}", &patch);

        // Write patch to a temporary file
        let patch_file = NamedTempFile::new()
            .context("Failed to create temporary file for patch")?;
        fs::write(patch_file.path(), patch)
            .context("Failed to write patch to temporary file")?;

        println!("{:?}", patch_file.path());
        

        // Apply the patch using git apply from the repository root
        let output = Command::new("git")
            .arg("apply")
            .arg(patch_file.path())
            .current_dir(&git_root_path)
            .output()
            .context("Failed to execute git apply")?;

        // Print stderr if there's any output (warnings, etc.)
        if !output.stderr.is_empty() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("git apply stderr: {}", stderr);
        }

        // Print stdout if there's any output
        if !output.stdout.is_empty() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            println!("git apply stdout: {}", stdout);
        }

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to apply patch. Exit code: {}. Error: {}", output.status.code().unwrap_or(-1), stderr);
        }

        println!("Patch applied successfully.");
    } else {
        println!("\nLast agent message:\n{}\n", response.output.last_message);
        println!("Patch:\n{}", response.output.patch);
    }

    Ok(())
}
