use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use git2::{
    build::{CheckoutBuilder, RepoBuilder},
    ApplyLocation, BranchType, Commit, Cred, CredentialType, Diff, DiffFormat, DiffOptions,
    ErrorCode, FetchOptions, IndexAddOption, PushOptions, RemoteCallbacks, Repository,
    RevparseMode, Status, StatusOptions,
};
use metis_common::{patches::GitOid, EnvGuard};

fn repo_for_path(path: &Path) -> Result<Repository> {
    Repository::discover(path).with_context(|| {
        format!(
            "failed to open git repository at or above {}",
            path.display()
        )
    })
}

pub fn repository_root(start: Option<&Path>) -> Result<PathBuf> {
    let start = start.unwrap_or_else(|| Path::new("."));
    let repo = repo_for_path(start)?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| anyhow!("failed to resolve git workdir for {}", start.display()))?;
    Ok(workdir.to_path_buf())
}

pub fn workdir_diff(repo_root: &Path) -> Result<String> {
    let repo = repo_for_path(repo_root)?;
    let head_tree = repo.head().ok().and_then(|head| head.peel_to_tree().ok());

    let mut staged_opts = DiffOptions::new();
    staged_opts.show_binary(true);
    let staged = repo
        .diff_tree_to_index(head_tree.as_ref(), None, Some(&mut staged_opts))
        .context("failed to compute staged diff")?;

    let mut opts = DiffOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true)
        .show_binary(true);
    let unstaged = repo
        .diff_index_to_workdir(None, Some(&mut opts))
        .context("failed to compute unstaged diff")?;

    let mut pieces = Vec::new();
    let staged_text = diff_to_string(&staged)?;
    if !staged_text.trim().is_empty() {
        pieces.push(staged_text);
    }
    let unstaged_text = diff_to_string(&unstaged)?;
    if !unstaged_text.trim().is_empty() {
        pieces.push(unstaged_text);
    }

    Ok(pieces.join("\n"))
}

pub fn diff_commit_range(repo_root: &Path, commit_range: &str) -> Result<String> {
    let repo = repo_for_path(repo_root)?;
    let trimmed = commit_range.trim();
    if trimmed.is_empty() {
        bail!("commit range must not be empty");
    }

    let rev_spec = repo
        .revparse(trimmed)
        .with_context(|| format!("failed to parse commit range '{trimmed}'"))?;
    if !rev_spec.mode().contains(RevparseMode::RANGE) {
        bail!("commit range '{trimmed}' must be specified in '<base>..<head>' format");
    }

    let base = rev_spec
        .from()
        .ok_or_else(|| anyhow!("commit range '{trimmed}' is missing a base revision"))?;
    let head = rev_spec
        .to()
        .ok_or_else(|| anyhow!("commit range '{trimmed}' is missing a head revision"))?;

    let base_tree = base
        .peel_to_tree()
        .with_context(|| format!("failed to resolve base tree for '{trimmed}'"))?;
    let head_tree = head
        .peel_to_tree()
        .with_context(|| format!("failed to resolve head tree for '{trimmed}'"))?;

    let mut opts = DiffOptions::new();
    opts.show_binary(true);
    let diff = repo
        .diff_tree_to_tree(Some(&base_tree), Some(&head_tree), Some(&mut opts))
        .with_context(|| format!("failed to compute diff for commit range '{trimmed}'"))?;

    diff_to_string(&diff)
}

pub fn apply_patch(repo_root: &Path, patch: &str) -> Result<()> {
    let repo = repo_for_path(repo_root)?;
    let diff = Diff::from_buffer(patch.as_bytes()).context("failed to parse patch contents")?;
    repo.apply(&diff, ApplyLocation::WorkDir, None)
        .context("failed to apply patch to workdir")?;
    Ok(())
}

pub fn clone_repo(url: &str, rev: &str, dest: &Path, github_token: Option<&str>) -> Result<()> {
    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(remote_callbacks(github_token));
    let mut checkout = CheckoutBuilder::new();
    checkout.safe();
    let repo = RepoBuilder::new()
        .fetch_options(fetch_opts)
        .with_checkout(checkout)
        .clone(url, dest)
        .with_context(|| format!("failed to clone repository from {url}"))?;

    fetch_revision(&repo, rev, github_token)
        .with_context(|| format!("failed to fetch revision '{rev}' from {url}"))?;
    checkout_revision(&repo, rev)?;
    Ok(())
}

fn fetch_revision(repo: &Repository, rev: &str, github_token: Option<&str>) -> Result<()> {
    if rev == "HEAD" || rev == "main" || rev.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Ok(());
    }

    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(remote_callbacks(github_token));

    let mut remote = repo
        .find_remote("origin")
        .context("failed to find origin remote")?;

    let refspecs = if rev.starts_with("refs/") {
        vec![rev.to_string()]
    } else {
        vec![
            format!("refs/heads/{rev}:refs/heads/{rev}"),
            format!("refs/tags/{rev}:refs/tags/{rev}"),
        ]
    };

    remote
        .fetch(&refspecs, Some(&mut fetch_opts), None)
        .context("failed to fetch revision from remote")?;

    Ok(())
}

pub fn configure_repo(repo_root: &Path, name: &str, email: &str) -> Result<()> {
    let repo = repo_for_path(repo_root)?;
    let mut config = repo
        .config()
        .context("failed to load git config for repository")?;
    config
        .set_str("user.name", name)
        .context("failed to set git user.name")?;
    config
        .set_str("user.email", email)
        .context("failed to set git user.email")?;
    Ok(())
}

pub fn resolve_head_oid(repo_root: &Path) -> Result<Option<GitOid>> {
    let repo = repo_for_path(repo_root)?;
    let head = repo.head();
    let result = match head {
        Ok(reference) => {
            let oid = reference
                .target()
                .ok_or_else(|| anyhow!("HEAD does not point to a commit"))?;
            Ok(Some(GitOid::new(oid)))
        }
        Err(err) if err.code() == ErrorCode::UnbornBranch || err.code() == ErrorCode::NotFound => {
            Ok(None)
        }
        Err(err) => Err(err).context("failed to resolve HEAD commit"),
    };
    result
}

pub fn current_branch(repo_root: &Path) -> Result<String> {
    let repo = repo_for_path(repo_root)?;
    let head = match repo.head() {
        Ok(head) => head,
        Err(err) if err.code() == ErrorCode::UnbornBranch || err.code() == ErrorCode::NotFound => {
            return Ok("HEAD".to_string());
        }
        Err(err) => return Err(err).context("failed to resolve current branch"),
    };

    Ok(head.shorthand().unwrap_or("HEAD").to_string())
}

pub fn branch_exists(repo_root: &Path, branch: &str) -> Result<bool> {
    let repo = repo_for_path(repo_root)?;
    let exists = repo.find_branch(branch, BranchType::Local).is_ok();
    Ok(exists)
}

pub fn checkout_new_branch(repo_root: &Path, branch: &str) -> Result<()> {
    let repo = repo_for_path(repo_root)?;
    let head_commit = repo
        .head()
        .context("failed to resolve HEAD for new branch")?
        .peel_to_commit()
        .context("failed to peel HEAD to commit for new branch")?;
    repo.branch(branch, &head_commit, false)
        .with_context(|| format!("failed to create branch '{branch}'"))?;
    repo.set_head(&format!("refs/heads/{branch}"))
        .with_context(|| format!("failed to set HEAD to branch '{branch}'"))?;
    let mut checkout = CheckoutBuilder::new();
    checkout.safe();
    repo.checkout_head(Some(&mut checkout))
        .context("failed to checkout new branch")?;
    Ok(())
}

pub fn stage_all_changes(repo_root: &Path) -> Result<()> {
    let repo = repo_for_path(repo_root)?;
    let mut index = repo.index().context("failed to open repository index")?;
    index
        .add_all(["*"], IndexAddOption::DEFAULT, None)
        .context("failed to stage changes")?;
    index.write().context("failed to write staged changes")?;
    Ok(())
}

pub fn ensure_staged_changes(repo_root: &Path) -> Result<()> {
    let repo = repo_for_path(repo_root)?;
    let index = repo.index().context("failed to open repository index")?;
    let base_tree = repo.head().ok().and_then(|head| head.peel_to_tree().ok());
    let mut diff_opts = DiffOptions::new();
    diff_opts.show_binary(true);
    let diff = repo
        .diff_tree_to_index(base_tree.as_ref(), Some(&index), Some(&mut diff_opts))
        .context("failed to compute staged diff")?;

    if diff.deltas().len() == 0 {
        bail!("No staged changes to commit for GitHub PR");
    }

    Ok(())
}

pub fn commit_changes(repo_root: &Path, message: &str) -> Result<()> {
    let repo = repo_for_path(repo_root)?;
    let signature = repo
        .signature()
        .context("failed to resolve git signature for commit")?;
    let mut index = repo.index().context("failed to open repository index")?;
    let tree_id = index
        .write_tree()
        .context("failed to write tree for commit")?;
    let tree = repo
        .find_tree(tree_id)
        .context("failed to load tree for commit")?;

    let parents: Vec<Commit> = repo
        .head()
        .ok()
        .and_then(|head| head.peel_to_commit().ok())
        .into_iter()
        .collect();
    let parent_refs: Vec<&Commit> = parents.iter().collect();

    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        message,
        &tree,
        &parent_refs,
    )
    .context("failed to create commit")?;

    Ok(())
}

pub fn push_branch(repo_root: &Path, branch: &str, github_token: Option<&str>) -> Result<()> {
    let repo = repo_for_path(repo_root)?;
    let mut remote = repo
        .find_remote("origin")
        .context("failed to find 'origin' remote")?;
    let callbacks = remote_callbacks(github_token);
    let mut push_options = PushOptions::new();
    push_options.remote_callbacks(callbacks);

    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    remote
        .push(&[refspec.as_str()], Some(&mut push_options))
        .with_context(|| format!("failed to push branch '{branch}' to origin"))?;

    if let Ok(mut local_branch) = repo.find_branch(branch, BranchType::Local) {
        let _ = local_branch.set_upstream(Some(&format!("origin/{branch}")));
    }

    Ok(())
}

fn diff_to_string(diff: &Diff) -> Result<String> {
    let _no_color = disable_diff_color();
    let mut output = String::new();
    diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
        output.push(line.origin());
        output.push_str(&String::from_utf8_lossy(line.content()));
        true
    })
    .context("failed to render diff as patch text")?;
    Ok(output)
}

fn disable_diff_color() -> EnvGuard {
    let git_config_parameters = env::var("GIT_CONFIG_PARAMETERS")
        .map(|existing| format!("{existing}\ncolor.ui=never"))
        .unwrap_or_else(|_| "color.ui=never".to_string());
    EnvGuard::set(&[
        ("NO_COLOR", Some("1")),
        ("CLICOLOR_FORCE", Some("0")),
        ("FORCE_COLOR", Some("0")),
        (
            "GIT_CONFIG_PARAMETERS",
            Some(git_config_parameters.as_str()),
        ),
        ("GIT_DIFF_OPTS", Some("--no-color")),
    ])
}

fn remote_callbacks(github_token: Option<&str>) -> RemoteCallbacks<'static> {
    let token = github_token.map(|value| value.to_string());
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(move |_url, username_from_url, allowed| {
        if let Some(token) = token.as_deref() {
            if allowed.contains(CredentialType::USER_PASS_PLAINTEXT) {
                return Cred::userpass_plaintext("x-access-token", token);
            }
        }

        if allowed.contains(CredentialType::SSH_KEY) {
            if let Some(username) = username_from_url {
                if let Ok(cred) = Cred::ssh_key_from_agent(username) {
                    return Ok(cred);
                }
            }
        }

        Cred::default()
    });
    callbacks
}

fn checkout_revision(repo: &Repository, rev: &str) -> Result<()> {
    let (object, reference) = repo
        .revparse_ext(rev)
        .with_context(|| format!("failed to resolve revision '{rev}'"))?;
    let mut checkout = CheckoutBuilder::new();
    checkout.safe();
    repo.checkout_tree(&object, Some(&mut checkout))
        .with_context(|| format!("failed to checkout revision '{rev}'"))?;

    if let Some(reference) = reference {
        if let Some(name) = reference.name() {
            repo.set_head(name)
                .with_context(|| format!("failed to set HEAD to '{rev}'"))?;
            return Ok(());
        }
    }

    repo.set_head_detached(object.id())
        .with_context(|| format!("failed to detach HEAD at '{rev}'"))?;
    Ok(())
}

pub fn has_uncommitted_changes(repo_root: &Path) -> Result<bool> {
    let repo = repo_for_path(repo_root)?;
    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_unmodified(false);
    let statuses = repo
        .statuses(Some(&mut opts))
        .context("failed to inspect working tree status")?;
    Ok(statuses
        .iter()
        .any(|entry| entry.status() != Status::CURRENT))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use anyhow::{Context, Result};
    use git2::{Repository, Signature};
    use tempfile::tempdir;

    use super::workdir_diff;
    use metis_common::EnvGuard;

    #[test]
    fn workdir_diff_omits_color_codes_even_when_forced() -> Result<()> {
        let tempdir = tempdir().context("failed to create tempdir for diff test")?;
        let repo = Repository::init(tempdir.path()).context("failed to init repo")?;
        let signature =
            Signature::now("Diff Tester", "diff.tester@example.com").context("signature")?;

        let file_path = tempdir.path().join("file.txt");
        std::fs::write(&file_path, "first line\n")?;
        stage_and_commit(&repo, &file_path, &signature, "init")?;

        std::fs::write(&file_path, "first line\nsecond line\n")?;

        let _color_env = EnvGuard::set(&[
            ("TERM", Some("xterm-256color")),
            ("COLORTERM", Some("truecolor")),
            ("CLICOLOR_FORCE", Some("1")),
            ("FORCE_COLOR", Some("1")),
            ("GIT_CONFIG_PARAMETERS", Some("color.ui=always")),
        ]);

        let diff = workdir_diff(tempdir.path())?;
        assert!(
            !diff.contains('\u{1b}'),
            "diff unexpectedly contained ANSI escapes: {diff:?}"
        );

        Ok(())
    }

    fn stage_and_commit(
        repo: &Repository,
        path: &Path,
        signature: &Signature<'_>,
        message: &str,
    ) -> Result<()> {
        let workdir = repo
            .workdir()
            .context("repository does not have a working directory")?;
        let relative_buf;
        let relative = if path.is_absolute() {
            let workdir_real = workdir
                .canonicalize()
                .context("failed to canonicalize repository root")?;
            let path_real = path
                .canonicalize()
                .context("failed to canonicalize path for staging")?;
            relative_buf = path_real
                .strip_prefix(&workdir_real)
                .context("path not within repository")?
                .to_path_buf();
            relative_buf.as_path()
        } else {
            path
        };
        let mut index = repo.index().context("failed to open index")?;
        index
            .add_path(relative)
            .context("failed to add path to index")?;
        let tree_id = index.write_tree().context("failed to write tree")?;
        let tree = repo.find_tree(tree_id).context("failed to find tree")?;
        repo.commit(Some("HEAD"), signature, signature, message, &tree, &[])
            .context("failed to commit")?;
        Ok(())
    }
}
