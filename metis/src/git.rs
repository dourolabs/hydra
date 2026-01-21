use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use git2::{
    build::{CheckoutBuilder, RepoBuilder},
    ApplyLocation, BranchType, Commit, Cred, CredentialType, Diff, DiffFormat, DiffOptions,
    ErrorCode, FetchOptions, IndexAddOption, PushOptions, RemoteCallbacks, Repository,
    RevparseMode, Status, StatusOptions,
};
use metis_common::patches::GitOid;

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

    checkout_revision(&repo, rev)?;
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
            Ok(Some(GitOid(oid)))
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
            return Ok("HEAD".to_string())
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
    let mut output = String::new();
    diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
        output.push(line.origin());
        output.push_str(&String::from_utf8_lossy(line.content()));
        true
    })
    .context("failed to render diff as patch text")?;
    Ok(strip_ansi_codes(&output))
}

fn strip_ansi_codes(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    for code in chars.by_ref() {
                        if ('@'..='~').contains(&code) {
                            break;
                        }
                    }
                    continue;
                }
                Some(']') => {
                    chars.next();
                    loop {
                        match chars.next() {
                            Some('\u{7}') => break,
                            Some('\u{1b}') if matches!(chars.peek(), Some('\\')) => {
                                chars.next();
                                break;
                            }
                            Some(_) => continue,
                            None => break,
                        }
                    }
                    continue;
                }
                _ => {}
            }
        }

        result.push(ch);
    }
    result
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
    use super::strip_ansi_codes;

    #[test]
    fn strip_ansi_removes_escape_sequences() {
        let colored = "line\n\u{1b}[31mremoved\u{1b}[0m\n\u{1b}[32madded\u{1b}[0m\n";
        let cleaned = strip_ansi_codes(colored);
        assert_eq!(cleaned, "line\nremoved\nadded\n");
    }

    #[test]
    fn strip_ansi_preserves_plain_text() {
        let plain = "line\nanother\n";
        assert_eq!(strip_ansi_codes(plain), plain);
    }
}
