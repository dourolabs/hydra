use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{anyhow, bail, Context, Result};
use hydra_common::constants::ENV_HYDRA_SERVER_URL;
use tokio::{io::AsyncWriteExt, process::Command};

use crate::command::output::CommandContext;

const CHAT_PRIMER: &str = r#"
You are the "hydra chat" issue-management assistant. Your role is to help the user manage
their work in Metis: creating issues, checking pending tasks, reviewing patches and design
documents, and managing issue states. You can run shell commands and should use the `hydra`
CLI to answer questions and take actions.

## Primary behaviors

### 1. Delegate work via issues — do not do work yourself
If the user asks for something to be done (e.g., "fix the login bug", "add a feature"),
create an issue for it — do NOT attempt the work yourself.
- Confirm the issue description with the user before creating it.
- Create the issue: `hydra issues create "<description>"`

### 2. Show pending work for the user
- Identify the current user: `hydra users info` (no arguments — returns the logged-in user).
- List their assigned issues: `hydra issues list --assignee <username>`
- Filter by status if useful: `hydra issues list --assignee <username> --status open`

### 3. Help review patches and design documents

**Patches:**
- List patches: `hydra patches list`
- Read a patch: `hydra patches describe <PATCH_ID>`
- Submit a review: `hydra patches review <PATCH_ID> --author <username> --contents '<review text>' [--approve]`
- Help the user read the patch, understand changes, and compose a review.

**Design documents:**
Documents are reviewed through a tracking issue, NOT directly on the document.
- Read the document: `hydra documents get --path /designs/<slug>.md`
- If the design is approved, close the review issue:
  `hydra issues update <review-issue-id> --status closed`
- If the design is rejected, fail the review issue with feedback:
  `hydra issues update <review-issue-id> --status failed --progress 'Feedback: ...'`

### 4. Manage rejected / failed states
If the user wants to reject an issue that is part of a broader plan:
- `hydra issues update <ISSUE_ID> --status failed --progress 'Rejected: <user feedback>'`
- Warn the user: marking an issue as failed triggers the parent issue's agent to replan
  around the rejection, and any child issues that depend on the failed issue will be dropped.
- Always explain this consequence and get confirmation before acting.

### 5. Investigate issue status via jobs
When the user asks "what's going on with issue X?":
- Look up jobs: `hydra jobs list --from <ISSUE_ID>`
- Check logs: `hydra jobs logs <JOB_ID>` (or `hydra jobs logs <ISSUE_ID>` for the latest job)

## CLI quick reference

Issues:   hydra issues list | describe | create | update
Patches:  hydra patches list | describe | create | review | update
Documents: hydra documents list | get | put | sync | push
Sessions: hydra sessions list | logs | create | kill
Users:    hydra users info

## Guidelines
1. Always use the CLI to answer questions — do not guess.
2. Ask for confirmation before mutating data (creating issues, submitting reviews, changing statuses).
3. Keep responses concise and reference evidence gathered from CLI output.
"#;

const INTERACTIVE_GREETING: &str =
    "The user will chat with you live. Greet them and briefly explain that you can help with issue management — creating issues, checking pending work, reviewing patches and documents, and managing issue states. Wait for their first instruction before acting.";

pub async fn run(
    server_url: &str,
    prompt: Option<String>,
    model: Option<String>,
    full_auto: bool,
    _context: &CommandContext,
) -> Result<()> {
    let working_dir = env::current_dir().context("failed to resolve current directory")?;
    let hydra_bin_dir = resolve_current_hydra_dir()?;
    let path_override = prepend_path_with(&hydra_bin_dir)?;

    match prompt {
        Some(prompt) => {
            run_noninteractive(
                &working_dir,
                server_url,
                &prompt,
                model.as_deref(),
                full_auto,
                &path_override,
            )
            .await
        }
        None => {
            run_interactive(
                &working_dir,
                server_url,
                model.as_deref(),
                full_auto,
                &path_override,
            )
            .await
        }
    }
}

async fn run_interactive(
    working_dir: &Path,
    server_url: &str,
    model: Option<&str>,
    full_auto: bool,
    path_override: &OsString,
) -> Result<()> {
    let prompt = build_prompt(INTERACTIVE_GREETING, server_url);

    let mut cmd = Command::new("codex");
    if full_auto {
        cmd.arg("--full-auto");
    }
    cmd.arg("--cd");
    cmd.arg(working_dir);
    if let Some(model) = model {
        cmd.arg("--model");
        cmd.arg(model);
    }
    // enable network access for the hydra command.
    // TODO: this sandboxing lets codex also mess with files in the current dir, which is weird.
    cmd.arg("-c");
    cmd.arg("sandbox_workspace_write.network_access=true");
    cmd.arg("--sandbox");
    cmd.arg("workspace-write");
    cmd.arg(prompt);
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    cmd.env(ENV_HYDRA_SERVER_URL, server_url);
    cmd.env("PATH", path_override);

    let status = cmd
        .status()
        .await
        .context("failed to start interactive codex session")?;
    if !status.success() {
        bail!("codex exited with status {status}");
    }

    Ok(())
}

async fn run_noninteractive(
    working_dir: &Path,
    server_url: &str,
    prompt: &str,
    model: Option<&str>,
    full_auto: bool,
    path_override: &OsString,
) -> Result<()> {
    let prompt = build_prompt(prompt, server_url);

    let mut cmd = Command::new("codex");
    cmd.arg("exec");
    if full_auto {
        cmd.arg("--full-auto");
    }
    cmd.arg("--cd");
    cmd.arg(working_dir);
    if let Some(model) = &model {
        cmd.arg("--model");
        cmd.arg(model);
    }
    // enable network access for the hydra command.
    // TODO: this sandboxing lets codex also mess with files in the current dir, which is weird.
    cmd.arg("-c");
    cmd.arg("sandbox_workspace_write.network_access=true");
    cmd.arg("--sandbox");
    cmd.arg("workspace-write");
    cmd.arg("-");
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    cmd.env(ENV_HYDRA_SERVER_URL, server_url);
    cmd.env("PATH", path_override);

    let mut child = cmd.spawn().context("failed to start codex chat session")?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open stdin for codex process"))?;
        stdin
            .write_all(prompt.as_bytes())
            .await
            .context("failed to send prompt to codex")?;
    }

    let status = child
        .wait()
        .await
        .context("failed waiting for codex chat process to finish")?;
    if !status.success() {
        bail!("codex exited with status {status}");
    }

    Ok(())
}

fn build_prompt(user_prompt: &str, server_url: &str) -> String {
    format!(
        "{primer}\n\nMetis server URL: {server_url}\n\nUser request:\n{user_prompt}\n",
        primer = CHAT_PRIMER.trim(),
        server_url = server_url.trim(),
        user_prompt = user_prompt.trim()
    )
}

fn resolve_current_hydra_dir() -> Result<PathBuf> {
    let exe = env::current_exe().context("failed to resolve running hydra binary")?;
    exe.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("running hydra binary has no parent directory"))
}

fn prepend_path_with(dir: &Path) -> Result<OsString> {
    let mut entries = vec![dir.to_path_buf()];
    if let Some(existing) = env::var_os("PATH") {
        entries.extend(env::split_paths(&existing));
    }
    env::join_paths(entries).context("failed to construct PATH for codex session")
}

#[cfg(test)]
mod tests {
    use super::build_prompt;

    #[test]
    fn prompt_includes_context() {
        let prompt = build_prompt("list jobs", "http://example.com");
        assert!(prompt.contains("issue-management assistant"));
        assert!(prompt.contains("http://example.com"));
        assert!(prompt.contains("list jobs"));
    }
}
