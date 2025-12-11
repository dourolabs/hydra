use std::{
    fs,
    io::Cursor,
    path::{Component, Path, PathBuf},
    process::Command,
};

use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use flate2::read::GzDecoder;
use metis_common::jobs::{Bundle, ParentContext, WorkerContext};
use tar::Archive;

use crate::client::MetisClientInterface;

pub async fn run(client: &dyn MetisClientInterface, job: String, dest: PathBuf) -> Result<()> {
    let WorkerContext {
        request_context,
        parents,
        setup,
        ..
    } = client.get_job_context(&job).await?;
    ensure_clean_destination(&dest)?;
    match request_context {
        Bundle::None => {
            fs::create_dir_all(&dest).with_context(|| format!("failed to create {dest:?}"))?;
        }
        Bundle::TarGz { archive_base64 } => {
            extract_tar_gz_base64(&archive_base64, &dest)?;
        }
        Bundle::GitRepository { url, rev } => {
            clone_git_repo(&url, &rev, &dest)?;
        }
        Bundle::GitBundle { bundle_base64 } => {
            clone_from_git_bundle_base64(&bundle_base64, &dest)?;
        }
    }
    write_parent_outputs(&parents, &dest)?;
    run_setup_commands(&setup, &dest)?;
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
                clone_git_repo(url, rev, &parent_dir)?;
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

fn clone_git_repo(url: &str, rev: &str, dest: &Path) -> Result<()> {
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

fn run_setup_commands(commands: &[String], working_dir: &Path) -> Result<()> {
    if commands.is_empty() {
        return Ok(());
    }

    for (idx, command) in commands.iter().enumerate() {
        let status = Command::new("bash")
            .arg("-c")
            .arg(command)
            .current_dir(working_dir)
            .status()
            .with_context(|| format!("failed to execute setup command {}: {}", idx + 1, command))?;
        if !status.success() {
            return Err(anyhow!(
                "setup command {} failed with status {}: {}",
                idx + 1,
                status,
                command
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use metis_common::job_outputs::JobOutputPayload;
    use std::{collections::HashMap, path::PathBuf};

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

        write_parent_outputs(&parents, tempdir.path())?;

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

        let err = write_parent_outputs(&parents, tempdir.path()).unwrap_err();
        assert!(
            err.to_string().contains("parent alias"),
            "expected alias validation error, got {err:?}"
        );

        let parents_dir = tempdir.path().join(".metis").join("parents");
        assert!(!parents_dir.join("../escape").exists());
    }
}
