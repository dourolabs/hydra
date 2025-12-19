use crate::client::MetisClientInterface;
use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as Base64Engine;
use base64::Engine;
use flate2::write::GzEncoder;
use flate2::Compression;
use metis_common::{job_outputs::JobOutputPayload, jobs::Bundle};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};
use tar::Builder;

pub async fn run(client: &dyn MetisClientInterface, job: String) -> Result<()> {
    let job_id = job.trim();
    if job_id.is_empty() {
        bail!("Job ID must not be empty.");
    }

    // Get cleanup commands from the job context
    let context = client.get_job_context(&job_id).await?;
    run_cleanup_commands(&context.cleanup, &context.variables)?;

    // Create patch file from git changes (excluding .metis directory)
    create_patch_file()?;

    let (last_message_file, patch_file, output_dir) = resolve_output_paths();

    let last_message = fs::read_to_string(&last_message_file).with_context(|| {
        format!(
            "failed to read last message output at '{}'",
            last_message_file.display()
        )
    })?;
    let patch = fs::read_to_string(&patch_file)
        .with_context(|| format!("failed to read patch output at '{}'", patch_file.display()))?;

    let bundle = create_output_bundle(&output_dir).with_context(|| {
        format!(
            "failed to create bundle from output directory '{}'",
            output_dir.display()
        )
    })?;

    let payload = JobOutputPayload {
        last_message,
        patch,
        bundle,
    };
    println!("Setting output for job '{job_id}' via metis-server…");
    let response = client.set_job_output(job_id, &payload).await?;
    println!(
        "Output set for job '{}'. Stored last message length: {}, patch length: {}",
        response.job_id,
        response.output.last_message.len(),
        response.output.patch.len()
    );
    Ok(())
}

fn resolve_output_paths() -> (PathBuf, PathBuf, PathBuf) {
    let output_dir = PathBuf::from(".metis").join("output");
    let last_message_file = output_dir.join("output.txt");
    let patch_file = output_dir.join("changes.patch");
    (last_message_file, patch_file, output_dir)
}

fn create_output_bundle(output_dir: &Path) -> Result<Bundle> {
    if !output_dir.exists() {
        return Ok(Bundle::None);
    }
    if !output_dir.is_dir() {
        bail!("'{}' is not a directory", output_dir.display());
    }

    // Create tar archive
    let mut tar_archive = Vec::new();
    {
        let mut builder = Builder::new(&mut tar_archive);
        builder
            .append_dir_all(".", output_dir)
            .with_context(|| format!("failed to archive directory '{}'", output_dir.display()))?;
        builder
            .finish()
            .context("failed to finalize output directory archive")?;
    }

    // Compress with gzip
    let mut gz_encoder = GzEncoder::new(Vec::new(), Compression::default());
    gz_encoder
        .write_all(&tar_archive)
        .context("failed to compress archive with gzip")?;
    let compressed = gz_encoder
        .finish()
        .context("failed to finalize gzip compression")?;

    // Base64 encode
    Ok(Bundle::TarGz {
        archive_base64: Base64Engine.encode(compressed),
    })
}

fn run_cleanup_commands(commands: &[String], variables: &HashMap<String, String>) -> Result<()> {
    if commands.is_empty() {
        return Ok(());
    }

    let current_dir = std::env::current_dir().context("failed to get current working directory")?;

    for (idx, command) in commands.iter().enumerate() {
        let status = Command::new("bash")
            .arg("-c")
            .arg(command)
            .current_dir(&current_dir)
            .envs(variables)
            .status()
            .with_context(|| {
                format!("failed to execute cleanup command {}: {}", idx + 1, command)
            })?;
        if !status.success() {
            return Err(anyhow!(
                "cleanup command {} failed with status {}: {}",
                idx + 1,
                status,
                command
            ));
        }
    }
    Ok(())
}

fn create_patch_file() -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to get current working directory")?;
    let patch_file = current_dir.join(".metis").join("output").join("changes.patch");

    // Ensure the output directory exists
    if let Some(parent) = patch_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory at {parent:?}"))?;
    }

    // Stage all changes excluding .metis directory
    let status = Command::new("git")
        .args(["add", "-A", "--", ".", ":!.metis/**"])
        .current_dir(&current_dir)
        .status()
        .context("failed to spawn git add")?;
    if !status.success() {
        return Err(anyhow!("git add failed with status {status}"));
    }

    // Create patch file from staged changes
    // Note: git diff returns exit code 1 when there are no changes (normal case)
    let output = Command::new("git")
        .args(["diff", "--cached", "--", ".", ":!.metis/**"])
        .current_dir(&current_dir)
        .output()
        .context("failed to spawn git diff")?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(anyhow!("git diff failed with status {}", output.status));
    }

    fs::write(&patch_file, &output.stdout)
        .with_context(|| format!("failed to write patch file to {patch_file:?}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use tempfile::tempdir;

    struct CwdGuard(std::path::PathBuf);

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = env::set_current_dir(&self.0);
        }
    }

    #[test]
    fn run_cleanup_commands_injects_variables_into_environment() -> Result<()> {
        let original_cwd = env::current_dir().context("failed to get current working directory")?;
        let guard = CwdGuard(original_cwd);
        let tempdir = tempdir().context("failed to create tempdir for test")?;
        env::set_current_dir(tempdir.path()).context("failed to change working directory")?;

        let mut variables = HashMap::new();
        variables.insert("CLEANUP_SECRET".to_string(), "cleanup-value".to_string());

        run_cleanup_commands(
            &[r#"echo "$CLEANUP_SECRET" > cleanup_env.txt"#.to_string()],
            &variables,
        )?;

        let output = std::fs::read_to_string(tempdir.path().join("cleanup_env.txt"))?;
        assert_eq!(output.trim(), "cleanup-value");
        assert!(
            env::var("CLEANUP_SECRET").is_err(),
            "cleanup variables must not leak into the parent process"
        );

        drop(guard);
        Ok(())
    }
}
