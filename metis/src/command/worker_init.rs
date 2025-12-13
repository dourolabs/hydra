use std::{
    collections::HashMap,
    env,
    fs,
    io::{Cursor, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
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
        variables,
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
    configure_git_identity(&dest, &variables)?;
    switch_to_worker_branch(&dest, &variables)?;
    run_setup_commands(&setup, &dest, &variables)?;
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

fn configure_git_identity(dest: &Path, variables: &HashMap<String, String>) -> Result<()> {
    if !dest.join(".git").exists() {
        return Ok(());
    }

    let repo = dest
        .to_str()
        .ok_or_else(|| anyhow!("destination path contains invalid UTF-8"))?;
    let user_name = resolve_variable("GIT_USER_NAME", variables)
        .unwrap_or_else(|| "metis-bot".to_string());
    let user_email = resolve_variable("GIT_USER_EMAIL", variables)
        .unwrap_or_else(|| "metis-bot@dourolabs.com".to_string());

    for (key, value) in [("user.name", user_name), ("user.email", user_email)] {
        let status = Command::new("git")
            .args(["-C", repo, "config", key, &value])
            .status()
            .with_context(|| format!("failed to spawn git config for {key}"))?;
        if !status.success() {
            return Err(anyhow!("git config {key} failed with status {status}"));
        }
    }

    Ok(())
}

fn switch_to_worker_branch(dest: &Path, variables: &HashMap<String, String>) -> Result<()> {
    if !dest.join(".git").exists() {
        return Ok(());
    }

    let repo = dest
        .to_str()
        .ok_or_else(|| anyhow!("destination path contains invalid UTF-8"))?;
    let metis_id = resolve_variable("METIS_ID", variables)
        .ok_or_else(|| anyhow!("METIS_ID is required to select the worker branch"))?;
    let branch_name = format!("metis-{metis_id}");

    let status = Command::new("git")
        .args(["-C", repo, "checkout", "-B", &branch_name])
        .status()
        .context("failed to spawn git checkout for worker branch")?;
    if !status.success() {
        return Err(anyhow!(
            "git checkout failed with status {status} when creating {branch_name}"
        ));
    }

    Ok(())
}

fn resolve_variable(key: &str, variables: &HashMap<String, String>) -> Option<String> {
    env::var(key)
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            variables
                .get(key)
                .filter(|value| !value.is_empty())
                .cloned()
        })
}

fn run_setup_commands(
    commands: &[String],
    working_dir: &Path,
    variables: &HashMap<String, String>,
) -> Result<()> {
    if commands.is_empty() {
        return Ok(());
    }

    for (idx, command) in commands.iter().enumerate() {
        let status = Command::new("bash")
            .arg("-c")
            .arg(command)
            .current_dir(working_dir)
            .envs(variables)
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
    use std::{collections::HashMap, env, path::PathBuf};

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

    #[test]
    fn run_setup_commands_injects_variables_into_environment() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        let mut variables = HashMap::new();
        variables.insert("SECRET_TOKEN".to_string(), "super-secret".to_string());

        run_setup_commands(
            &[r#"echo "$SECRET_TOKEN" > env_output.txt"#.to_string()],
            tempdir.path(),
            &variables,
        )?;

        let output = std::fs::read_to_string(tempdir.path().join("env_output.txt"))?;
        assert_eq!(output.trim(), "super-secret");
        assert!(
            std::env::var("SECRET_TOKEN").is_err(),
            "setup variables must not leak into the parent process"
        );

        Ok(())
    }

    #[test]
    fn configure_git_identity_sets_local_config_from_env() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        Command::new("git")
            .args(["init", tempdir.path().to_str().unwrap()])
            .status()
            .context("failed to initialize git repository for test")?;

        let _name_guard = EnvGuard::set("GIT_USER_NAME", Some("cli-bot"));
        let _email_guard = EnvGuard::set("GIT_USER_EMAIL", Some("cli-bot@example.com"));

        configure_git_identity(tempdir.path(), &HashMap::new())?;

        let user_name = Command::new("git")
            .args(["-C", tempdir.path().to_str().unwrap(), "config", "user.name"])
            .output()
            .context("failed to read user.name from git config")?;
        assert!(user_name.status.success(), "git config user.name failed");
        assert_eq!(
            String::from_utf8_lossy(&user_name.stdout).trim(),
            "cli-bot"
        );

        let user_email = Command::new("git")
            .args(["-C", tempdir.path().to_str().unwrap(), "config", "user.email"])
            .output()
            .context("failed to read user.email from git config")?;
        assert!(
            user_email.status.success(),
            "git config user.email failed"
        );
        assert_eq!(
            String::from_utf8_lossy(&user_email.stdout).trim(),
            "cli-bot@example.com"
        );

        Ok(())
    }

    #[test]
    fn switch_to_worker_branch_prefers_context_variables() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        Command::new("git")
            .args(["init", tempdir.path().to_str().unwrap()])
            .status()
            .context("failed to initialize git repository for branch test")?;

        let _metis_guard = EnvGuard::set("METIS_ID", None);
        let mut variables = HashMap::new();
        variables.insert("METIS_ID".to_string(), "branch-id".to_string());

        switch_to_worker_branch(tempdir.path(), &variables)?;

        let current_branch = Command::new("git")
            .args(["-C", tempdir.path().to_str().unwrap(), "symbolic-ref", "--short", "HEAD"])
            .output()
            .context("failed to read current branch name")?;
        assert!(current_branch.status.success(), "git rev-parse failed");
        assert_eq!(
            String::from_utf8_lossy(&current_branch.stdout).trim(),
            "metis-branch-id"
        );

        Ok(())
    }

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: Option<&str>) -> Self {
            let original = env::var(key).ok();
            match value {
                Some(value) => env::set_var(key, value),
                None => env::remove_var(key),
            }
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.original {
                env::set_var(self.key, value);
            } else {
                env::remove_var(self.key);
            }
        }
    }
}
