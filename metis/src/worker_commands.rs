use std::{collections::HashMap, path::Path, process::Stdio, time::Instant};

use crate::claude_formatter::StreamFormatter;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use metis_common::constants::{
    ENV_ANTHROPIC_API_KEY, ENV_CLAUDE_CODE_OAUTH_TOKEN, ENV_OPENAI_API_KEY,
};
use tokio::{
    fs,
    io::{self, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

/// Grace period after the main process exits before killing the process group.
const PROCESS_GROUP_GRACE_PERIOD: std::time::Duration = std::time::Duration::from_secs(5);

/// Time to wait for SIGTERM to take effect before sending SIGKILL.
const SIGTERM_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Timeout for stdout/stderr pipe reads after process group kill.
/// If pipes don't EOF within this duration, we drop the handles and move on.
const PIPE_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Sends SIGTERM then SIGKILL to a process group.
///
/// The `pgid` is the process group ID (same as the leader's PID when spawned
/// with `process_group(0)`). Signals are sent via `kill(-pgid, sig)` so they
/// reach every process in the group.
#[cfg(unix)]
async fn kill_process_group(pgid: u32) {
    let neg_pgid = -(pgid as i32);

    // SIGTERM — give processes a chance to exit cleanly
    // SAFETY: kill with a negative pid signals the process group.
    unsafe {
        libc::kill(neg_pgid, libc::SIGTERM);
    }

    tokio::time::sleep(SIGTERM_WAIT).await;

    // SIGKILL — force-kill anything still alive
    unsafe {
        libc::kill(neg_pgid, libc::SIGKILL);
    }
}

/// Walks `/proc` to find all descendant processes of `ancestor_pid` and kills them.
///
/// This catches processes that escaped the original process group by calling
/// `setsid()`, `setpgid()`, or being reparented. We walk the process tree by
/// reading `/proc/<pid>/stat` to find each process's parent PID, then signal
/// all descendants with SIGKILL.
#[cfg(unix)]
fn kill_descendant_processes(ancestor_pid: u32) {
    use std::collections::{HashMap, HashSet, VecDeque};

    let our_pid = std::process::id();

    // Build a map of pid -> parent_pid from /proc
    let mut parent_map: HashMap<u32, u32> = HashMap::new();
    let proc_dir = match std::fs::read_dir("/proc") {
        Ok(d) => d,
        Err(e) => {
            eprintln!("kill_descendant_processes: failed to read /proc: {e}");
            return;
        }
    };

    for entry in proc_dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let pid: u32 = match name_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };

        // Read /proc/<pid>/stat to get ppid (field 4, 1-indexed)
        let stat_path = format!("/proc/{pid}/stat");
        let stat_contents = match std::fs::read_to_string(&stat_path) {
            Ok(c) => c,
            Err(_) => continue, // process may have exited
        };

        // The comm field (field 2) is in parens and may contain spaces,
        // so find the last ')' and parse fields after it.
        if let Some(after_comm) = stat_contents.rfind(')') {
            let rest = &stat_contents[after_comm + 2..]; // skip ") "
            let fields: Vec<&str> = rest.split_whitespace().collect();
            // fields[0] = state, fields[1] = ppid
            if let Some(ppid_str) = fields.get(1) {
                if let Ok(ppid) = ppid_str.parse::<u32>() {
                    parent_map.insert(pid, ppid);
                }
            }
        }
    }

    // BFS from ancestor_pid to find all descendants
    let mut descendants = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(ancestor_pid);
    while let Some(pid) = queue.pop_front() {
        for (&child, &ppid) in &parent_map {
            if ppid == pid && child != our_pid && descendants.insert(child) {
                queue.push_back(child);
            }
        }
    }

    if descendants.is_empty() {
        return;
    }

    eprintln!(
        "kill_descendant_processes: killing {} descendant process(es) of PID {ancestor_pid}: {:?}",
        descendants.len(),
        descendants
    );

    for pid in &descendants {
        // SAFETY: sending SIGKILL to a specific process
        unsafe {
            libc::kill(*pid as i32, libc::SIGKILL);
        }
    }
}

#[async_trait]
pub trait WorkerCommands: Send + Sync {
    async fn run(
        &self,
        prompt: &str,
        model: Option<&str>,
        openai_api_key: Option<String>,
        anthropic_api_key: Option<String>,
        claude_code_oauth_token: Option<String>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
    ) -> Result<String>;
}

pub struct CodexCommands;
pub struct ClaudeCommands;

pub struct ModelAwareCommands {
    codex: CodexCommands,
    claude: ClaudeCommands,
}

impl Default for ModelAwareCommands {
    fn default() -> Self {
        Self {
            codex: CodexCommands,
            claude: ClaudeCommands,
        }
    }
}

fn is_claude_model(model: &str) -> bool {
    let lc = model.to_ascii_lowercase();
    lc.contains("claude") || lc.contains("haiku") || lc.contains("sonnet") || lc.contains("opus")
}

impl CodexCommands {
    async fn login(&self, openai_api_key: Option<&str>) -> Result<()> {
        let openai_api_key = openai_api_key.map(str::to_owned).ok_or_else(|| {
            anyhow!("{ENV_OPENAI_API_KEY} must be provided via --openai-api-key or environment")
        })?;

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
                .await
                .with_context(|| format!("failed to write {ENV_OPENAI_API_KEY} to codex login"))?;
        }

        let status = login_cmd
            .wait()
            .await
            .context("failed waiting for codex login to finish")?;
        if !status.success() {
            return Err(anyhow!("codex login failed with status {status}"));
        }

        Ok(())
    }

    async fn run_codex(
        prompt: &str,
        model: Option<&str>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
    ) -> Result<String> {
        if let Some(dir) = output_path.parent() {
            fs::create_dir_all(dir)
                .await
                .with_context(|| format!("failed to create codex output directory {dir:?}"))?;
        }

        let mut command = Command::new("codex");
        command
            .args([
                "exec",
                "--color",
                "always",
                "--skip-git-repo-check",
                "-o",
                output_path
                    .to_str()
                    .expect("codex output path should be valid UTF-8"),
                "--dangerously-bypass-approvals-and-sandbox",
            ])
            .current_dir(working_dir)
            .envs(env);
        #[cfg(unix)]
        command.process_group(0);
        if let Some(model) = model {
            command.arg("--model");
            command.arg(model);
        }
        command.arg(prompt);

        let status = command
            .status()
            .await
            .context("failed to spawn codex command")?;

        if !status.success() {
            return Err(anyhow!("codex command failed with status {status}"));
        }

        fs::read_to_string(output_path)
            .await
            .with_context(|| format!("failed to read codex output from {output_path:?}"))
    }
}

#[async_trait]
impl WorkerCommands for CodexCommands {
    async fn run(
        &self,
        prompt: &str,
        model: Option<&str>,
        openai_api_key: Option<String>,
        _anthropic_api_key: Option<String>,
        _claude_code_oauth_token: Option<String>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
    ) -> Result<String> {
        self.login(openai_api_key.as_deref()).await?;
        Self::run_codex(prompt, model, working_dir, env, output_path)
            .await
            .with_context(|| "failed to execute codex for worker context")
    }
}

impl ClaudeCommands {
    async fn run_claude(
        prompt: &str,
        model: Option<&str>,
        anthropic_api_key: Option<String>,
        claude_code_oauth_token: Option<String>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
    ) -> Result<String> {
        if let Some(dir) = output_path.parent() {
            fs::create_dir_all(dir)
                .await
                .with_context(|| format!("failed to create claude output directory {dir:?}"))?;
        }

        let anthropic_api_key = anthropic_api_key.filter(|value| !value.trim().is_empty());
        let claude_code_oauth_token =
            claude_code_oauth_token.filter(|value| !value.trim().is_empty());

        if anthropic_api_key.is_none() && claude_code_oauth_token.is_none() {
            return Err(anyhow!(
                "Either {ENV_CLAUDE_CODE_OAUTH_TOKEN} or {ENV_ANTHROPIC_API_KEY} must be provided via CLI flags or environment"
            ));
        }

        let mut command = Command::new("claude");
        command.arg("--print");
        command.arg("--dangerously-skip-permissions");
        command.arg("--verbose");
        command.arg("--output-format");
        command.arg("stream-json");
        if let Some(model) = model {
            command.arg("--model");
            command.arg(model);
        }
        command.current_dir(working_dir).envs(env);
        #[cfg(unix)]
        command.process_group(0);
        if let Some(key) = anthropic_api_key.as_ref() {
            command.env(ENV_ANTHROPIC_API_KEY, key);
        }
        if let Some(token) = claude_code_oauth_token.as_ref() {
            command.env(ENV_CLAUDE_CODE_OAUTH_TOKEN, token);
        }

        command.arg(prompt);

        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to spawn claude command")?;

        let spawn_time = Instant::now();
        let pid = child.id().unwrap_or(0);
        println!("Claude process spawned (PID: {pid})");

        #[cfg(unix)]
        let child_pgid = child.id();

        let child_stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("failed to capture stdout for claude command"))?;
        let child_stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("failed to capture stderr for claude command"))?;

        let stderr_handle = tokio::spawn(async move {
            let mut stderr_buf = Vec::new();
            tokio::io::BufReader::new(child_stderr)
                .read_to_end(&mut stderr_buf)
                .await
                .context("failed to read claude stderr")?;
            Ok::<Vec<u8>, anyhow::Error>(stderr_buf)
        });

        // Spawn stdout reading into a separate task so we can race it against
        // the child process exiting. This prevents hanging when a background
        // process inherits the stdout pipe fd.
        let mut stdout_handle = tokio::spawn(async move {
            let mut formatter = StreamFormatter::new();
            let mut reader = BufReader::new(child_stdout);
            let mut stdout_buf = String::new();
            let mut stdout_writer = io::stdout();
            let mut line = String::new();
            loop {
                line.clear();
                let read = reader
                    .read_line(&mut line)
                    .await
                    .context("failed to read claude stdout")?;
                if read == 0 {
                    let elapsed = spawn_time.elapsed().as_secs_f64();
                    println!("Claude stdout EOF reached (PID: {pid}, elapsed: {elapsed:.2}s)");
                    break;
                }
                for formatted in formatter.handle_line(&line) {
                    stdout_writer
                        .write_all(formatted.as_bytes())
                        .await
                        .context("failed to stream claude stdout")?;
                    stdout_writer
                        .flush()
                        .await
                        .context("failed to flush claude stdout")?;
                    stdout_buf.push_str(&formatted);
                }
            }
            let last_message = formatter.last_assistant_text().map(str::to_owned);
            Ok::<(String, Option<String>), anyhow::Error>((stdout_buf, last_message))
        });

        // Wait for the main claude process to exit.
        println!("Waiting for claude process to exit (PID: {pid})…");
        let status = child
            .wait()
            .await
            .context("failed waiting for claude command to finish")?;
        let wait_elapsed = spawn_time.elapsed().as_secs_f64();
        println!(
            "Claude process exited (PID: {pid}, status: {status}, elapsed: {wait_elapsed:.2}s)"
        );

        // The main process has exited. Give stdout a grace period to reach EOF
        // (it will if no background processes inherited the pipe).
        let stdout_result =
            tokio::time::timeout(PROCESS_GROUP_GRACE_PERIOD, &mut stdout_handle).await;

        // If stdout didn't finish, kill the process group to close inherited fds.
        #[cfg(unix)]
        if stdout_result.is_err() {
            if let Some(pgid) = child_pgid {
                eprintln!(
                    "claude process exited but stdout pipe still open; \
                     killing process group {pgid}"
                );
                kill_process_group(pgid).await;

                // Kill any descendant processes that escaped the process group
                // (e.g., via setsid/setpgid or new process groups from pnpm/Node.js).
                kill_descendant_processes(pgid);
            }
        }

        // Await stdout with a timeout so we never hang indefinitely.
        let stdout_result = tokio::time::timeout(PIPE_READ_TIMEOUT, stdout_handle).await;
        let (stdout_buf, last_message) = match stdout_result {
            Ok(join_result) => join_result.context("failed to join claude stdout task")??,
            Err(_) => {
                let timeout = PIPE_READ_TIMEOUT;
                eprintln!(
                    "stdout pipe read timed out after {timeout:?} — \
                     dropping handle and proceeding with partial output"
                );
                (String::new(), None)
            }
        };

        // Await stderr with a timeout so we never hang indefinitely.
        let stderr_result = tokio::time::timeout(PIPE_READ_TIMEOUT, stderr_handle).await;
        let stderr_buf = match stderr_result {
            Ok(join_result) => join_result.context("failed to join claude stderr task")??,
            Err(_) => {
                let timeout = PIPE_READ_TIMEOUT;
                eprintln!(
                    "stderr pipe read timed out after {timeout:?} — \
                     dropping handle and proceeding without stderr"
                );
                Vec::new()
            }
        };

        if !status.success() {
            return Err(anyhow!(
                "claude command failed with status {}. stdout: {}. stderr: {}",
                status,
                stdout_buf,
                String::from_utf8_lossy(&stderr_buf)
            ));
        }

        fs::write(output_path, stdout_buf.as_bytes())
            .await
            .with_context(|| format!("failed to write claude output to {output_path:?}"))?;

        let last_message = last_message.unwrap_or(stdout_buf);
        Ok(last_message)
    }
}

#[async_trait]
impl WorkerCommands for ClaudeCommands {
    async fn run(
        &self,
        prompt: &str,
        model: Option<&str>,
        _openai_api_key: Option<String>,
        anthropic_api_key: Option<String>,
        claude_code_oauth_token: Option<String>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
    ) -> Result<String> {
        Self::run_claude(
            prompt,
            model,
            anthropic_api_key,
            claude_code_oauth_token,
            working_dir,
            env,
            output_path,
        )
        .await
        .with_context(|| "failed to execute claude for worker context")
    }
}

#[async_trait]
impl WorkerCommands for ModelAwareCommands {
    async fn run(
        &self,
        prompt: &str,
        model: Option<&str>,
        openai_api_key: Option<String>,
        anthropic_api_key: Option<String>,
        claude_code_oauth_token: Option<String>,
        working_dir: &Path,
        env: &HashMap<String, String>,
        output_path: &Path,
    ) -> Result<String> {
        match model.filter(|value| is_claude_model(value)) {
            Some(_) => {
                self.claude
                    .run(
                        prompt,
                        model,
                        openai_api_key,
                        anthropic_api_key,
                        claude_code_oauth_token.clone(),
                        working_dir,
                        env,
                        output_path,
                    )
                    .await
            }
            None => {
                self.codex
                    .run(
                        prompt,
                        model,
                        openai_api_key,
                        anthropic_api_key,
                        claude_code_oauth_token,
                        working_dir,
                        env,
                        output_path,
                    )
                    .await
            }
        }
    }
}
