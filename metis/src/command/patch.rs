use crate::{client::MetisClient, config::AppConfig};
use anyhow::{bail, Context, Result};
use std::{fs, io::Write, path::PathBuf, process::Command};
use tempfile::NamedTempFile;

/// ANSI color codes
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";

/// Pretty-print a patch with color coding (green for additions, red for deletions).
fn pretty_print_patch(patch: &str) {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();

    for line in patch.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            // Addition line (but not the +++ header)
            writeln!(handle, "{GREEN}{line}{RESET}").unwrap();
        } else if line.starts_with('-') && !line.starts_with("---") {
            // Deletion line (but not the --- header)
            writeln!(handle, "{RED}{line}{RESET}").unwrap();
        } else {
            // Context lines, headers, etc.
            writeln!(handle, "{line}").unwrap();
        }
    }
}

pub async fn run(config: &AppConfig, job: String, apply: bool) -> Result<()> {
    let job_id = job.trim();
    if job_id.is_empty() {
        bail!("Job ID must not be empty.");
    }
    let job_id = job_id.to_string();
    let client = MetisClient::from_config(config)?;

    println!("Fetching patch for job '{job_id}' via metis-server…");

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

        // Show the patch with color coding before applying
        println!("\nPatch to be applied:\n");
        pretty_print_patch(&patch);

        // Write patch to a temporary file
        let patch_file =
            NamedTempFile::new().context("Failed to create temporary file for patch")?;
        fs::write(patch_file.path(), patch).context("Failed to write patch to temporary file")?;

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
            eprintln!("git apply stderr: {stderr}");
        }

        // Print stdout if there's any output
        if !output.stdout.is_empty() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            println!("git apply stdout: {stdout}");
        }

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "Failed to apply patch. Exit code: {}. Error: {}",
                output.status.code().unwrap_or(-1),
                stderr
            );
        }

        println!("Patch applied successfully.");
    } else {
        if !response.output.last_message.is_empty() {
            println!("\nLast agent message:\n{}\n", response.output.last_message);
        }

        if !response.output.patch.is_empty() {
            println!("Patch:\n");
            pretty_print_patch(&response.output.patch);
        } else {
            println!("\nNo patch available for this job.");
        }
    }

    Ok(())
}
