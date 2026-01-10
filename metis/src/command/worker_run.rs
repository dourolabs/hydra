use std::{
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use flate2::read::GzDecoder;
use metis_common::artifacts::{Artifact, UpsertArtifactRequest};
use metis_common::job_status::JobStatusUpdate;
use metis_common::MetisId;
use metis_common::{
    constants::{ENV_GH_TOKEN, ENV_OPENAI_API_KEY},
    jobs::{Bundle, WorkerContext},
};
use tar::Archive;

use crate::client::MetisClientInterface;
use crate::constants;
use crate::exec::eval_with_closure_unwrapping;

pub async fn run(client: &dyn MetisClientInterface, job: MetisId, dest: PathBuf) -> Result<()> {
    let job_id = job.trim().to_string();
    if job_id.is_empty() {
        bail!("job ID must not be empty");
    }

    let WorkerContext {
        request_context,
        variables,
        program,
        params,
        ..
    } = client.get_job_context(&job_id).await?;
    // Startup tasks: set up context
    ensure_clean_destination(&dest)?;
    let github_token = variables.get(ENV_GH_TOKEN).map(String::as_str);
    match request_context {
        Bundle::None => {
            fs::create_dir_all(&dest).with_context(|| format!("failed to create {dest:?}"))?;
        }
        Bundle::TarGz { archive_base64 } => {
            extract_tar_gz_base64(&archive_base64, &dest)?;
        }
        Bundle::GitRepository { url, rev } => {
            clone_git_repo(&url, &rev, &dest, github_token)?;
        }
        Bundle::GitBundle { bundle_base64 } => {
            clone_from_git_bundle_base64(&bundle_base64, &dest)?;
        }
    }
    create_output_directory(&dest)?;

    login_codex()?;
    configure_git_repo(&dest)?;

    let _ = eval_with_closure_unwrapping(&program, params, &variables)
        .await
        .with_context(|| "failed to execute Rhai program from worker context")?;

    // Submit job status (merge of worker-submit functionality)
    submit_job_status(client, &job_id, &dest).await?;

    Ok(())
}

fn ensure_clean_destination(dest: &Path) -> Result<()> {
    if dest.exists() {
        let mut entries =
            fs::read_dir(dest).with_context(|| format!("failed to read directory {dest:?}"))?;
        if entries.next().is_some() {
            return Err(anyhow!(
                "destination {dest:?} is not empty; choose an empty or new directory"
            ));
        }
        Ok(())
    } else {
        fs::create_dir_all(dest).with_context(|| format!("failed to create {dest:?}"))
    }
}

fn extract_tar_gz_base64(archive_base64: &str, dest: &Path) -> Result<()> {
    let bytes = BASE64_STANDARD
        .decode(archive_base64)
        .context("failed to base64-decode archive")?;
    let gz = GzDecoder::new(Cursor::new(bytes));
    let mut archive = Archive::new(gz);
    archive
        .unpack(dest)
        .with_context(|| format!("failed to extract archive into {dest:?}"))?;
    Ok(())
}

fn clone_git_repo(url: &str, rev: &str, dest: &Path, github_token: Option<&str>) -> Result<()> {
    if let Some(token) = github_token {
        authenticate_github(token)?;
    }

    let status = Command::new("git")
        .args(["clone", "--no-checkout", url, dest.to_str().unwrap()])
        .status()
        .context("failed to spawn git clone")?;
    if !status.success() {
        return Err(anyhow!("git clone failed with status {status}"));
    }

    let status = Command::new("git")
        .args(["-C", dest.to_str().unwrap(), "checkout", rev])
        .status()
        .context("failed to spawn git checkout")?;
    if !status.success() {
        return Err(anyhow!("git checkout failed with status {status}"));
    }
    Ok(())
}

fn authenticate_github(token: &str) -> Result<()> {
    // Authenticate the GitHub CLI and configure git credentials for private repo access.
    let mut login_cmd = Command::new("gh")
        .args(["auth", "login", "--with-token"])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn gh auth login")?;

    {
        let mut stdin = login_cmd
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open stdin for gh auth login"))?;
        stdin
            .write_all(format!("{token}\n").as_bytes())
            .with_context(|| format!("failed to write {ENV_GH_TOKEN} to gh auth login"))?;
    }

    let status = login_cmd
        .wait()
        .context("failed waiting for gh auth login to finish")?;
    if !status.success() {
        return Err(anyhow!("gh auth login failed with status {status}"));
    }

    let status = Command::new("gh")
        .args(["auth", "setup-git"])
        .status()
        .context("failed to spawn gh auth setup-git")?;
    if !status.success() {
        return Err(anyhow!("gh auth setup-git failed with status {status}"));
    }

    Ok(())
}

fn clone_from_git_bundle_base64(bundle_base64: &str, dest: &Path) -> Result<()> {
    let bytes = BASE64_STANDARD
        .decode(bundle_base64)
        .context("failed to base64-decode git bundle")?;
    let tmpdir = tempfile::Builder::new()
        .prefix("metis-bundle-")
        .tempdir()
        .context("failed to create temporary directory")?;
    let bundle_path = tmpdir.path().join("repo.bundle");
    fs::write(&bundle_path, bytes).context("failed to write git bundle to temp file")?;

    let status = Command::new("git")
        .args([
            "clone",
            bundle_path.to_str().unwrap(),
            dest.to_str().unwrap(),
        ])
        .status()
        .context("failed to spawn git clone from bundle")?;
    if !status.success() {
        return Err(anyhow!("git clone from bundle failed with status {status}"));
    }
    Ok(())
}

fn configure_git_repo(dest: &Path) -> Result<()> {
    let git_dir = dest.join(".git");
    if !git_dir.exists() {
        return Ok(());
    }

    let repo_path = dest
        .to_str()
        .ok_or_else(|| anyhow!("destination path contains invalid UTF-8"))?;

    let status = Command::new("git")
        .args(["-C", repo_path, "config", "user.name", "Metis Worker"])
        .status()
        .context("failed to set git user.name")?;
    if !status.success() {
        return Err(anyhow!("git config user.name failed with status {status}"));
    }

    let status = Command::new("git")
        .args([
            "-C",
            repo_path,
            "config",
            "user.email",
            "metis-worker@example.com",
        ])
        .status()
        .context("failed to set git user.email")?;
    if !status.success() {
        return Err(anyhow!("git config user.email failed with status {status}"));
    }

    Ok(())
}

fn login_codex() -> Result<()> {
    let openai_api_key = std::env::var(ENV_OPENAI_API_KEY)
        .with_context(|| format!("{ENV_OPENAI_API_KEY} is not set; unable to login Codex CLI"))?;

    let mut login_cmd = Command::new("codex")
        .args(["login", "--with-api-key"])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn codex login")?;

    {
        let mut stdin = login_cmd
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open stdin for codex login"))?;
        stdin
            .write_all(format!("{openai_api_key}\n").as_bytes())
            .with_context(|| format!("failed to write {ENV_OPENAI_API_KEY} to codex login"))?;
    }

    let status = login_cmd
        .wait()
        .context("failed waiting for codex login to finish")?;
    if !status.success() {
        return Err(anyhow!("codex login failed with status {status}"));
    }

    Ok(())
}

fn create_output_directory(dest: &Path) -> Result<()> {
    let output_dir = dest.join(constants::METIS_DIR).join(constants::OUTPUT_DIR);
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create output directory at {output_dir:?}"))?;
    Ok(())
}

async fn submit_job_status(
    client: &dyn MetisClientInterface,
    job: &MetisId,
    dest: &Path,
) -> Result<()> {
    if job.is_empty() {
        bail!("Job ID must not be empty.");
    }

    // Create patch file from git changes (excluding METIS_DIR directory)
    create_patch_file(dest)?;

    let (last_message_file, patch_file) = resolve_output_paths(dest);

    let last_message = fs::read_to_string(&last_message_file).with_context(|| {
        format!(
            "failed to read last message output at '{}'",
            last_message_file.display()
        )
    })?;
    let patch = fs::read_to_string(&patch_file)
        .with_context(|| format!("failed to read patch output at '{}'", patch_file.display()))?;

    client
        .create_artifact(&UpsertArtifactRequest {
            artifact: Artifact::Patch {
                diff: patch.clone(),
                description: last_message.clone(),
            },
            job_id: Some(job.clone()),
        })
        .await?;
    println!("Updating status for job '{job}' via metis-server…");
    let response = client
        .set_job_status(job, &JobStatusUpdate::Complete)
        .await?;
    println!(
        "Status updated for job '{}'. Stored last message length: {}, patch length: {}",
        response.job_id,
        last_message.len(),
        patch.len()
    );
    Ok(())
}

fn resolve_output_paths(dest: &Path) -> (PathBuf, PathBuf) {
    let output_dir = dest.join(constants::METIS_DIR).join(constants::OUTPUT_DIR);
    let last_message_file = output_dir.join(constants::OUTPUT_TXT_FILE);
    let patch_file = output_dir.join(constants::CHANGES_PATCH_FILE);
    (last_message_file, patch_file)
}

fn create_patch_file(dest: &Path) -> Result<()> {
    let patch_file = dest
        .join(constants::METIS_DIR)
        .join(constants::OUTPUT_DIR)
        .join(constants::CHANGES_PATCH_FILE);

    // Ensure the output directory exists
    if let Some(parent) = patch_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory at {parent:?}"))?;
    }

    let patch = create_patch_from_repo(dest)?;

    fs::write(&patch_file, patch.as_bytes())
        .with_context(|| format!("failed to write patch file to {patch_file:?}"))?;

    Ok(())
}

/// Create a unified diff from the repository at `dest`, staging all changes (including
/// untracked files) except for `.metis/**`, and returning the diff as a string.
pub fn create_patch_from_repo(dest: &Path) -> Result<String> {
    let temp_dir = tempfile::tempdir()
        .context("failed to create temporary directory for git index during patch creation")?;
    let temp_index_path = temp_dir.path().join("index");

    let patch = create_patch_with_index(dest, Some(temp_index_path.as_path()))?;
    Ok(patch)
}

fn create_patch_with_index(dest: &Path, index_file: Option<&Path>) -> Result<String> {
    stage_changes(dest, index_file)?;
    capture_cached_diff(dest, index_file)
}

fn stage_changes(dest: &Path, index_file: Option<&Path>) -> Result<()> {
    // Stage all changes excluding METIS_DIR directory
    // Note that we don't care if this fails, as it fails if there are no changes.
    git_command(dest, index_file)
        .args([
            "add",
            "-A",
            "--",
            ".",
            &format!(":!{}/**", constants::METIS_DIR),
        ])
        .status()
        .context("failed to spawn git add")?;

    Ok(())
}

fn capture_cached_diff(dest: &Path, index_file: Option<&Path>) -> Result<String> {
    // Create patch from staged changes
    // Note: git diff returns exit code 1 when there are no changes (normal case)
    let output = git_command(dest, index_file)
        .args([
            "diff",
            "--cached",
            "--",
            ".",
            &format!(":!{}/**", constants::METIS_DIR),
        ])
        .current_dir(dest)
        .output()
        .context("failed to spawn git diff")?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(anyhow!("git diff failed with status {}", output.status));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn git_command(dest: &Path, index_file: Option<&Path>) -> Command {
    let mut command = Command::new("git");
    command.current_dir(dest);
    if let Some(index) = index_file {
        command.env("GIT_INDEX_FILE", index);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    // Test helpers for create_patch_file tests
    fn init_git_repo(repo_path: &Path) -> Result<String> {
        let repo_str = repo_path
            .to_str()
            .ok_or_else(|| anyhow!("repo path contains invalid UTF-8"))?;

        Command::new("git")
            .args(["init", repo_str])
            .status()
            .context("failed to init git repo for test")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git init returned non-zero exit code"))?;

        Command::new("git")
            .args(["-C", repo_str, "config", "user.name", "Test User"])
            .status()
            .context("failed to set git user.name")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git config user.name returned non-zero exit code"))?;

        Command::new("git")
            .args(["-C", repo_str, "config", "user.email", "test@example.com"])
            .status()
            .context("failed to set git user.email")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git config user.email returned non-zero exit code"))?;

        Ok(repo_str.to_string())
    }

    fn create_initial_commit(
        repo_path: &Path,
        repo_str: &str,
        filename: &str,
        content: &str,
    ) -> Result<()> {
        std::fs::write(repo_path.join(filename), content)
            .with_context(|| format!("failed to write initial file {filename}"))?;

        Command::new("git")
            .args(["-C", repo_str, "add", filename])
            .status()
            .with_context(|| format!("failed to add initial file {filename}"))?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git add returned non-zero exit code"))?;

        Command::new("git")
            .args(["-C", repo_str, "commit", "-m", "initial commit"])
            .status()
            .context("failed to create initial commit")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git commit returned non-zero exit code"))?;

        Ok(())
    }

    fn setup_git_repo_with_initial_commit(repo_path: &Path) -> Result<String> {
        let repo_str = init_git_repo(repo_path)?;
        create_initial_commit(repo_path, &repo_str, "README.md", "initial content")?;
        Ok(repo_str)
    }

    fn read_patch_file(repo_path: &Path) -> Result<String> {
        let patch_file = repo_path
            .join(constants::METIS_DIR)
            .join(constants::OUTPUT_DIR)
            .join(constants::CHANGES_PATCH_FILE);
        fs::read_to_string(&patch_file)
            .with_context(|| format!("failed to read patch file at {}", patch_file.display()))
    }

    #[test]
    fn configure_git_repo_sets_user_config_and_branch() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        let repo_path = tempdir.path();
        let repo_str = repo_path
            .to_str()
            .ok_or_else(|| anyhow!("tempdir path contains invalid UTF-8"))?;

        Command::new("git")
            .args(["init", repo_str])
            .status()
            .context("failed to init git repo for test")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git init returned non-zero exit code"))?;
        Command::new("git")
            .args(["-C", repo_str, "config", "user.name", "Initial User"])
            .status()
            .context("failed to set initial git user.name")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git config user.name returned non-zero exit code"))?;
        Command::new("git")
            .args([
                "-C",
                repo_str,
                "config",
                "user.email",
                "initial@example.com",
            ])
            .status()
            .context("failed to set initial git user.email")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git config user.email returned non-zero exit code"))?;
        std::fs::write(repo_path.join("README.md"), "hello world")
            .context("failed to write initial file for git repo")?;
        Command::new("git")
            .args(["-C", repo_str, "add", "."])
            .status()
            .context("failed to add file for initial commit")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git add returned non-zero exit code"))?;
        Command::new("git")
            .args(["-C", repo_str, "commit", "-m", "init"])
            .status()
            .context("failed to create initial commit")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git commit returned non-zero exit code"))?;

        configure_git_repo(repo_path)?;

        let user_name = Command::new("git")
            .args(["-C", repo_str, "config", "user.name"])
            .output()
            .context("failed to read git user.name")?;
        assert!(user_name.status.success());
        assert_eq!(
            String::from_utf8_lossy(&user_name.stdout).trim(),
            "Metis Worker"
        );

        let user_email = Command::new("git")
            .args(["-C", repo_str, "config", "user.email"])
            .output()
            .context("failed to read git user.email")?;
        assert!(user_email.status.success());
        assert_eq!(
            String::from_utf8_lossy(&user_email.stdout).trim(),
            "metis-worker@example.com"
        );

        Ok(())
    }

    #[test]
    fn create_patch_file_includes_untracked_files() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        let repo_path = tempdir.path();

        setup_git_repo_with_initial_commit(repo_path)?;

        // Create new untracked files
        std::fs::write(repo_path.join("new_file.txt"), "new content")
            .context("failed to write new file")?;
        std::fs::create_dir_all(repo_path.join("src")).context("failed to create src directory")?;
        std::fs::write(repo_path.join("src").join("main.rs"), "fn main() {}")
            .context("failed to write main.rs")?;

        // Create patch file
        create_patch_file(repo_path)?;

        // Read and verify patch file
        let patch_content = read_patch_file(repo_path)?;

        // Verify new files are included in patch
        assert!(
            patch_content.contains("new_file.txt"),
            "patch should include new_file.txt"
        );
        assert!(
            patch_content.contains("new content"),
            "patch should include content of new_file.txt"
        );
        assert!(
            patch_content.contains("src/main.rs"),
            "patch should include src/main.rs"
        );
        assert!(
            patch_content.contains("fn main() {}"),
            "patch should include content of src/main.rs"
        );

        Ok(())
    }

    #[test]
    fn create_patch_file_excludes_metis_directory() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        let repo_path = tempdir.path();

        setup_git_repo_with_initial_commit(repo_path)?;

        // Create files in .metis directory (should be excluded)
        let metis_dir = repo_path.join(constants::METIS_DIR);
        std::fs::create_dir_all(&metis_dir).context("failed to create .metis directory")?;
        std::fs::write(metis_dir.join("internal_file.txt"), "internal content")
            .context("failed to write file in .metis")?;
        std::fs::create_dir_all(metis_dir.join("subdir"))
            .context("failed to create subdir in .metis")?;
        std::fs::write(
            metis_dir.join("subdir").join("nested.txt"),
            "nested content",
        )
        .context("failed to write nested file in .metis")?;

        // Also create a regular file that should be included
        std::fs::write(repo_path.join("regular_file.txt"), "regular content")
            .context("failed to write regular file")?;

        // Create patch file
        create_patch_file(repo_path)?;

        // Read and verify patch file
        let patch_content = read_patch_file(repo_path)?;

        // Verify .metis files are excluded from patch
        assert!(
            !patch_content.contains(".metis/internal_file.txt"),
            "patch should not include .metis/internal_file.txt"
        );
        assert!(
            !patch_content.contains("internal content"),
            "patch should not include content from .metis/internal_file.txt"
        );
        assert!(
            !patch_content.contains(".metis/subdir/nested.txt"),
            "patch should not include .metis/subdir/nested.txt"
        );
        assert!(
            !patch_content.contains("nested content"),
            "patch should not include content from .metis/subdir/nested.txt"
        );

        // Verify regular file is included
        assert!(
            patch_content.contains("regular_file.txt"),
            "patch should include regular_file.txt"
        );
        assert!(
            patch_content.contains("regular content"),
            "patch should include content of regular_file.txt"
        );

        Ok(())
    }

    #[test]
    fn create_patch_file_ignores_gitignored_paths() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        let repo_path = tempdir.path();
        let repo_str = setup_git_repo_with_initial_commit(repo_path)?;

        // Create .gitignore that ignores *.log files
        std::fs::write(repo_path.join(".gitignore"), "*.log\ntarget/\n")
            .context("failed to write .gitignore")?;
        Command::new("git")
            .args(["-C", &repo_str, "add", ".gitignore"])
            .status()
            .context("failed to add .gitignore")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git add .gitignore returned non-zero exit code"))?;
        Command::new("git")
            .args(["-C", &repo_str, "commit", "-m", "add .gitignore"])
            .status()
            .context("failed to commit .gitignore")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git commit .gitignore returned non-zero exit code"))?;

        // Create files that match .gitignore patterns (should be ignored)
        std::fs::write(repo_path.join("build.log"), "log content")
            .context("failed to write build.log")?;
        std::fs::create_dir_all(repo_path.join("target"))
            .context("failed to create target directory")?;
        std::fs::write(
            repo_path.join("target").join("artifact.bin"),
            "binary content",
        )
        .context("failed to write target/artifact.bin")?;

        // Create a file that should be included (not in .gitignore)
        std::fs::create_dir_all(repo_path.join("src")).context("failed to create src directory")?;
        std::fs::write(repo_path.join("src").join("main.rs"), "fn main() {}")
            .context("failed to write src/main.rs")?;

        // Create patch file
        create_patch_file(repo_path)?;

        // Read and verify patch file
        let patch_content = read_patch_file(repo_path)?;

        // Verify gitignored files are excluded from patch
        assert!(
            !patch_content.contains("build.log"),
            "patch should not include build.log (matched by *.log pattern)"
        );
        assert!(
            !patch_content.contains("log content"),
            "patch should not include content from build.log"
        );
        assert!(
            !patch_content.contains("target/artifact.bin"),
            "patch should not include target/artifact.bin (matched by target/ pattern)"
        );
        assert!(
            !patch_content.contains("binary content"),
            "patch should not include content from target/artifact.bin"
        );

        // Verify non-ignored file is included
        assert!(
            patch_content.contains("src/main.rs"),
            "patch should include src/main.rs (not in .gitignore)"
        );
        assert!(
            patch_content.contains("fn main() {}"),
            "patch should include content of src/main.rs"
        );

        Ok(())
    }

    #[test]
    fn create_patch_file_generates_empty_patch_when_no_changes() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        let repo_path = tempdir.path();

        setup_git_repo_with_initial_commit(repo_path)?;

        // No changes made after the commit - directory is clean

        // Create patch file
        create_patch_file(repo_path)?;

        // Read and verify patch file is empty
        let patch_content = read_patch_file(repo_path)?;

        // Verify patch is empty when there are no changes
        assert!(
            patch_content.is_empty(),
            "patch should be empty when directory has no changes, but got: {patch_content:?}"
        );

        Ok(())
    }
}
