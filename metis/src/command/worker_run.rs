use std::{
    fs,
    io::{Cursor, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use metis_common::job_outputs::JobOutputPayload;
use metis_common::jobs::{Bundle, ParentContext, WorkerContext};
use tar::{Archive, Builder};

use crate::client::MetisClientInterface;
use crate::exec::eval_with_closure_unwrapping;

pub async fn run(client: &dyn MetisClientInterface, job: String, dest: PathBuf) -> Result<()> {
    let WorkerContext {
        request_context,
        parents,
        variables,
        program,
        params,
        ..
    } = client.get_job_context(&job).await?;
    ensure_clean_destination(&dest)?;
    let github_token = variables.get("GH_TOKEN").map(String::as_str);
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
    write_parent_outputs(&parents, &dest, github_token)?;
    configure_git_repo(&dest)?;
    if let Some(program) = program {
        let _ = eval_with_closure_unwrapping(&program, params, &variables)
            .with_context(|| "failed to execute Rhai program from worker context")?;
    }

    // Startup tasks: codex login and create output directory
    login_codex()?;
    create_output_directory(&dest)?;

    // Submit job output (merge of worker-submit functionality)
    submit_job_output(client, &job, &dest).await?;

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

fn write_parent_outputs(
    parents: &std::collections::HashMap<String, ParentContext>,
    dest: &Path,
    github_token: Option<&str>,
) -> Result<()> {
    if parents.is_empty() {
        return Ok(());
    }

    let parents_dir = dest.join(".metis").join("parents");
    fs::create_dir_all(&parents_dir)
        .with_context(|| format!("failed to create parents directory at {parents_dir:?}"))?;

    for (metis_id, parent) in parents {
        let parent_dir = parents_dir.join(metis_id);
        fs::create_dir_all(&parent_dir)
            .with_context(|| format!("failed to create directory {parent_dir:?}"))?;

        match &parent.output.bundle {
            Bundle::None => {
                // Directory already created above, nothing to extract
            }
            Bundle::TarGz { archive_base64 } => {
                extract_tar_gz_base64(archive_base64, &parent_dir)?;
            }
            Bundle::GitRepository { url, rev } => {
                clone_git_repo(url, rev, &parent_dir, github_token)?;
            }
            Bundle::GitBundle { bundle_base64 } => {
                clone_from_git_bundle_base64(bundle_base64, &parent_dir)?;
            }
        }

        if let Some(name) = &parent.name {
            if !name.is_empty() && name != metis_id {
                let alias = validate_parent_alias(name).with_context(|| {
                    format!("parent alias must be a single path segment: {:?}", name)
                })?;
                let symlink_path = parents_dir.join(alias);
                create_symlink(Path::new(metis_id), &symlink_path).with_context(|| {
                    format!("failed to create symlink {symlink_path:?} -> {metis_id}")
                })?;
            }
        }
    }

    Ok(())
}

fn create_symlink(target: &Path, link: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).context("failed to create symlink")?;
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(target, link).context("failed to create symlink")?;
    }
    Ok(())
}

fn validate_parent_alias(alias: &str) -> Result<&str> {
    let mut components = Path::new(alias).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(alias),
        _ => Err(anyhow!(
            "invalid parent alias {:?}: must not contain separators, traversal, or prefixes",
            alias
        )),
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
            .context("failed to write GH_TOKEN to gh auth login")?;
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
    let openai_api_key = std::env::var("OPENAI_API_KEY")
        .context("OPENAI_API_KEY is not set; unable to login Codex CLI")?;

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
            .context("failed to write OPENAI_API_KEY to codex login")?;
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
    let output_dir = dest.join(".metis").join("output");
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create output directory at {output_dir:?}"))?;
    Ok(())
}

async fn submit_job_output(
    client: &dyn MetisClientInterface,
    job: &str,
    dest: &Path,
) -> Result<()> {
    let job_id = job.trim();
    if job_id.is_empty() {
        bail!("Job ID must not be empty.");
    }

    // Create patch file from git changes (excluding .metis directory)
    create_patch_file(dest)?;

    let (last_message_file, patch_file, output_dir) = resolve_output_paths(dest);

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

fn resolve_output_paths(dest: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let output_dir = dest.join(".metis").join("output");
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
        archive_base64: BASE64_STANDARD.encode(compressed),
    })
}

fn create_patch_file(dest: &Path) -> Result<()> {
    let patch_file = dest.join(".metis").join("output").join("changes.patch");

    // Ensure the output directory exists
    if let Some(parent) = patch_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory at {parent:?}"))?;
    }

    // Stage all changes excluding .metis directory
    let status = Command::new("git")
        .args(["add", "-A", "--", ".", ":!.metis/**"])
        .current_dir(dest)
        .status()
        .context("failed to spawn git add")?;
    if !status.success() {
        return Err(anyhow!("git add failed with status {status}"));
    }

    // Create patch file from staged changes
    // Note: git diff returns exit code 1 when there are no changes (normal case)
    let output = Command::new("git")
        .args(["diff", "--cached", "--", ".", ":!.metis/**"])
        .current_dir(dest)
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
    use metis_common::job_outputs::JobOutputPayload;
    use std::{collections::HashMap, path::PathBuf, process::Command};

    #[test]
    fn write_parent_outputs_creates_symlink_for_named_parent() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        let mut parents = HashMap::new();
        parents.insert(
            "parent-id".to_string(),
            ParentContext {
                name: Some("parent-name".to_string()),
                output: JobOutputPayload {
                    last_message: String::new(),
                    patch: String::new(),
                    bundle: Bundle::None,
                },
            },
        );

        write_parent_outputs(&parents, tempdir.path(), None)?;

        let parents_dir = tempdir.path().join(".metis").join("parents");
        assert!(parents_dir.join("parent-id").is_dir());

        let symlink_path = parents_dir.join("parent-name");
        let metadata = std::fs::symlink_metadata(&symlink_path)?;
        assert!(metadata.file_type().is_symlink());
        let target = std::fs::read_link(&symlink_path)?;
        assert_eq!(target, PathBuf::from("parent-id"));

        Ok(())
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
    fn write_parent_outputs_rejects_traversal_aliases() {
        let tempdir = tempfile::tempdir().expect("failed to create tempdir for test");
        let mut parents = HashMap::new();
        parents.insert(
            "parent-id".to_string(),
            ParentContext {
                name: Some("../escape".to_string()),
                output: JobOutputPayload {
                    last_message: String::new(),
                    patch: String::new(),
                    bundle: Bundle::None,
                },
            },
        );

        let err = write_parent_outputs(&parents, tempdir.path(), None).unwrap_err();
        assert!(
            err.to_string().contains("parent alias"),
            "expected alias validation error, got {err:?}"
        );

        let parents_dir = tempdir.path().join(".metis").join("parents");
        assert!(!parents_dir.join("../escape").exists());
    }

}
