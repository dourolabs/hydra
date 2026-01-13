use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{anyhow, bail, Context, Result};
use metis_common::constants::ENV_METIS_SERVER_URL;
use tokio::{io::AsyncWriteExt, process::Command};

use crate::config::AppConfig;

const CHAT_PRIMER: &str = r#"
You are Codex acting as the "metis chat" assistant. You can run shell commands in the
current workspace and should use the `metis` CLI as your primary tool. Helpful commands:

- `metis jobs --limit N` lists recent jobs in the current namespace.
- `metis logs <JOB_ID> [--watch]` streams job logs.
- `metis spawn ...` launches new jobs (confirm with the user before running destructive work).
- `metis kill <JOB_ID>` stops jobs.
- `metis issues <subcommand>` manages issues.
- `metis patches <subcommand>` inspects or applies patches.
- `metis worker-run <JOB_ID> <PATH>` fetches a job context locally.
- `metis run <SCRIPT>` executes Rhai automation helpers.

Guidelines:
1. Prefer answering questions by calling the CLI instead of guessing.
2. Show the commands you run and summarize the relevant parts of their output.
3. Ask for confirmation before taking destructive actions (spawning, killing, or mutating data).
4. Keep the final response concise and reference the evidence you gathered.
"#;

const INTERACTIVE_GREETING: &str =
    "The user will chat with you live. Greet them, explain you can run `metis` commands, and wait for their first instruction before acting.";

pub async fn run(
    config: &AppConfig,
    prompt: Option<String>,
    model: Option<String>,
    full_auto: bool,
) -> Result<()> {
    let working_dir = env::current_dir().context("failed to resolve current directory")?;
    let server_url = config.server.url.clone();
    let metis_bin_dir = resolve_current_metis_dir()?;
    let path_override = prepend_path_with(&metis_bin_dir)?;

    match prompt {
        Some(prompt) => {
            run_noninteractive(
                &working_dir,
                &server_url,
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
                &server_url,
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
    cmd.arg(prompt);
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    cmd.env(ENV_METIS_SERVER_URL, server_url);
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
    cmd.arg("-");
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    cmd.env(ENV_METIS_SERVER_URL, server_url);
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

fn resolve_current_metis_dir() -> Result<PathBuf> {
    let exe = env::current_exe().context("failed to resolve running metis binary")?;
    exe.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("running metis binary has no parent directory"))
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
        assert!(prompt.contains("Codex acting as"));
        assert!(prompt.contains("http://example.com"));
        assert!(prompt.contains("list jobs"));
    }
}
