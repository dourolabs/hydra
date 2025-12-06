use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use flate2::read::GzDecoder;
use metis_common::jobs::CreateJobRequestContext;
use tar::Archive;

use crate::{client::MetisClient, config::AppConfig};

pub async fn run(config: &AppConfig, job: String, dest: PathBuf) -> Result<()> {
    let client = MetisClient::from_config(config)?;
    let context = client.get_job_context(&job).await?;
    ensure_clean_destination(&dest)?;
    match context {
        CreateJobRequestContext::None => {
            fs::create_dir_all(&dest).with_context(|| format!("failed to create {dest:?}"))?;
        }
        CreateJobRequestContext::UploadDirectory { archive_base64 } => {
            extract_tar_gz_base64(&archive_base64, &dest)?;
        }
        CreateJobRequestContext::GitRepository { url, rev } => {
            clone_git_repo(&url, &rev, &dest)?;
        }
        CreateJobRequestContext::GitBundle { bundle_base64 } => {
            clone_from_git_bundle_base64(&bundle_base64, &dest)?;
        }
    }
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
