use std::{
    fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

use anyhow::{bail, ensure, Context, Result};
use clap::Subcommand;

use crate::{
    config::{self, expand_path},
    constants::DEFAULT_CONFIG_FILE,
};

/// Directory layout under ~/.metis/
const SERVER_DIR: &str = "~/.metis/server";
const SERVER_CONFIG_PATH: &str = "~/.metis/server/config.yaml";
const AUTH_TOKEN_PATH: &str = "~/.metis/server/auth-token";
const PID_FILE_PATH: &str = "~/.metis/server/metis-server.pid";
const LOG_DIR: &str = "~/.metis/server/logs";
const LOG_FILE_PATH: &str = "~/.metis/server/logs/metis-server.log";
const SERVER_DB_PATH: &str = "~/.metis/server/metis.db";
const BIN_DIR: &str = "~/.metis/bin";

const LOCAL_SERVER_URL: &str = "http://127.0.0.1:8080";

#[cfg(target_os = "macos")]
const LAUNCHD_LABEL: &str = "dev.metis.server";
#[cfg(target_os = "macos")]
const LAUNCHD_PLIST_PATH: &str = "~/Library/LaunchAgents/dev.metis.server.plist";

#[cfg(target_os = "linux")]
const SYSTEMD_UNIT_NAME: &str = "metis-server.service";

#[derive(Debug, Subcommand)]
pub enum ServerCommand {
    /// Initialize the local metis server (create config, start server, configure CLI).
    Init,
    /// Start the local metis server as a background daemon.
    Start,
    /// Stop the local metis server.
    Stop,
    /// Show the status of the local metis server.
    Status,
    /// Tail the local metis server logs.
    Logs {
        /// Number of lines to show (default: 50).
        #[arg(short = 'n', long, default_value_t = 50)]
        lines: usize,
        /// Follow log output (like tail -f).
        #[arg(short, long)]
        follow: bool,
    },
    /// Restart the local metis server (stop + start).
    Restart,
}

pub fn run(command: ServerCommand) -> Result<()> {
    match command {
        ServerCommand::Init => cmd_init(),
        ServerCommand::Start => cmd_start(),
        ServerCommand::Stop => cmd_stop(),
        ServerCommand::Status => cmd_status(),
        ServerCommand::Logs { lines, follow } => cmd_logs(lines, follow),
        ServerCommand::Restart => cmd_restart(),
    }
}

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

fn cmd_init() -> Result<()> {
    let server_dir = expand_path(SERVER_DIR);
    let config_path = expand_path(SERVER_CONFIG_PATH);

    if config_path.exists() {
        bail!(
            "Server already initialized (config exists at {}). \
             Run `metis server start` to start it.",
            config_path.display()
        );
    }

    // Prompt for GitHub PAT.
    let github_pat = prompt_github_pat()?;

    // Generate a random 32-byte encryption key (base64-encoded).
    let encryption_key = generate_encryption_key();

    // Create directory structure.
    let log_dir = expand_path(LOG_DIR);
    let bin_dir = expand_path(BIN_DIR);
    fs::create_dir_all(&server_dir)
        .with_context(|| format!("failed to create {}", server_dir.display()))?;
    fs::create_dir_all(&log_dir)
        .with_context(|| format!("failed to create {}", log_dir.display()))?;
    fs::create_dir_all(&bin_dir)
        .with_context(|| format!("failed to create {}", bin_dir.display()))?;

    // Write server config.
    let db_path = expand_path(SERVER_DB_PATH);
    let auth_token_path_expanded = expand_path(AUTH_TOKEN_PATH);
    let config_content = render_server_config(
        &encryption_key,
        &github_pat,
        &db_path,
        &auth_token_path_expanded,
    );
    fs::write(&config_path, &config_content)
        .with_context(|| format!("failed to write config to {}", config_path.display()))?;

    println!("Server config written to {}", config_path.display());

    // Start the server so it creates the local user and auth token.
    println!("Starting server...");
    start_server_process()?;

    // Wait for the auth token file to appear (the server writes it on startup).
    let token_path = expand_path(AUTH_TOKEN_PATH);
    let auth_token = wait_for_auth_token(&token_path)?;

    // Write the auth token to the CLI config.
    let cli_config_path = expand_path(Path::new(DEFAULT_CONFIG_FILE));
    config::store_auth_token(&cli_config_path, LOCAL_SERVER_URL, &auth_token)?;
    println!("CLI configured with auth token for {LOCAL_SERVER_URL}");

    println!();
    println!("Metis is running! Dashboard: {LOCAL_SERVER_URL}");
    println!();
    println!("Next steps:");
    println!("  metis issues list              # list issues");
    println!("  metis server status             # check server status");
    println!("  metis server logs --follow      # watch server logs");
    println!("  metis server stop               # stop the server");

    Ok(())
}

fn prompt_github_pat() -> Result<String> {
    eprint!("Enter your GitHub Personal Access Token (PAT): ");
    io::stderr().flush()?;
    let token = rpassword::prompt_password_stdout("").context("failed to read GitHub PAT")?;
    let token = token.trim().to_string();
    ensure!(!token.is_empty(), "GitHub PAT is required");
    Ok(token)
}

fn generate_encryption_key() -> String {
    let mut key = [0u8; 32];
    getrandom::getrandom(&mut key).expect("failed to generate random bytes from OS CSPRNG");
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(key)
}

fn render_server_config(
    encryption_key: &str,
    github_pat: &str,
    db_path: &Path,
    auth_token_path: &Path,
) -> String {
    format!(
        r#"# Metis server configuration (auto-generated by `metis server init`)
metis:
  METIS_SECRET_ENCRYPTION_KEY: "{encryption_key}"

auth_mode: local
github_token: "{github_pat}"
auth_token_file: "{auth_token_path}"

storage_backend: sqlite
sqlite_path: "{db_path}"

job_engine: local

job:
  default_image: "metis-worker:latest"
"#,
        db_path = db_path.display(),
        auth_token_path = auth_token_path.display(),
    )
}

fn wait_for_auth_token(token_path: &Path) -> Result<String> {
    // Poll for the token file up to 30 seconds.
    let max_wait = Duration::from_secs(30);
    let poll_interval = Duration::from_millis(500);
    let start = std::time::Instant::now();

    while start.elapsed() < max_wait {
        if token_path.exists() {
            let token = fs::read_to_string(token_path).with_context(|| {
                format!("failed to read auth token from {}", token_path.display())
            })?;
            let token = token.trim().to_string();
            if !token.is_empty() {
                return Ok(token);
            }
        }
        thread::sleep(poll_interval);
    }

    bail!(
        "Timed out waiting for auth token at {}. \
         The server may have failed to start — check logs with `metis server logs`.",
        token_path.display()
    );
}

// ---------------------------------------------------------------------------
// start
// ---------------------------------------------------------------------------

fn cmd_start() -> Result<()> {
    ensure_initialized()?;

    if is_server_running()? {
        println!("Server is already running.");
        return Ok(());
    }

    start_server_process()?;
    println!("Server started.");
    Ok(())
}

fn start_server_process() -> Result<()> {
    start_server_platform()
}

#[cfg(target_os = "macos")]
fn start_server_platform() -> Result<()> {
    start_with_launchd()
}

#[cfg(target_os = "linux")]
fn start_server_platform() -> Result<()> {
    start_with_systemd().or_else(|_| start_with_pid_file())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn start_server_platform() -> Result<()> {
    start_with_pid_file()
}

fn find_server_binary() -> Result<PathBuf> {
    // Look for metis-server at ~/.metis/bin/metis-server first, then in PATH.
    let local_bin = expand_path(BIN_DIR).join("metis-server");
    if local_bin.exists() {
        return Ok(local_bin);
    }

    which_server_binary()
}

fn which_server_binary() -> Result<PathBuf> {
    let output = Command::new("which")
        .arg("metis-server")
        .output()
        .context("failed to search PATH for metis-server")?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }

    bail!(
        "metis-server binary not found. \
         Install it at ~/.metis/bin/metis-server or ensure it is in your PATH."
    );
}

#[cfg(target_os = "macos")]
fn start_with_launchd() -> Result<()> {
    let server_binary = find_server_binary()?;
    let config_path = expand_path(SERVER_CONFIG_PATH);
    let log_file = expand_path(LOG_FILE_PATH);
    let plist_path = expand_path(LAUNCHD_PLIST_PATH);

    // Create the plist directory if needed.
    if let Some(parent) = plist_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LAUNCHD_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{server_binary}</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>METIS_CONFIG</key>
        <string>{config_path}</string>
    </dict>
    <key>RunAtLoad</key>
    <false/>
    <key>KeepAlive</key>
    <false/>
    <key>StandardOutPath</key>
    <string>{log_file}</string>
    <key>StandardErrorPath</key>
    <string>{log_file}</string>
</dict>
</plist>
"#,
        server_binary = server_binary.display(),
        config_path = config_path.display(),
        log_file = log_file.display(),
    );

    fs::write(&plist_path, plist_content)
        .with_context(|| format!("failed to write plist to {}", plist_path.display()))?;

    let status = Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist_path)
        .status()
        .context("failed to run launchctl load")?;

    ensure!(status.success(), "launchctl load failed");

    let status = Command::new("launchctl")
        .args(["start", LAUNCHD_LABEL])
        .status()
        .context("failed to run launchctl start")?;

    ensure!(status.success(), "launchctl start failed");

    Ok(())
}

#[cfg(target_os = "linux")]
fn start_with_systemd() -> Result<()> {
    let server_binary = find_server_binary()?;
    let config_path = expand_path(SERVER_CONFIG_PATH);

    let unit_dir = expand_path("~/.config/systemd/user");
    fs::create_dir_all(&unit_dir)
        .with_context(|| format!("failed to create {}", unit_dir.display()))?;

    let unit_path = unit_dir.join(SYSTEMD_UNIT_NAME);

    let unit_content = format!(
        r#"[Unit]
Description=Metis Server
After=network.target

[Service]
Type=simple
ExecStart={server_binary}
Environment=METIS_CONFIG={config_path}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"#,
        server_binary = server_binary.display(),
        config_path = config_path.display(),
    );

    fs::write(&unit_path, unit_content)
        .with_context(|| format!("failed to write systemd unit to {}", unit_path.display()))?;

    // Reload systemd to pick up the new/changed unit file.
    let reload = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    if reload.is_err() || !reload.unwrap().success() {
        bail!("systemctl --user daemon-reload failed; falling back to PID file");
    }

    let status = Command::new("systemctl")
        .args(["--user", "start", SYSTEMD_UNIT_NAME])
        .status()
        .context("failed to start systemd unit")?;

    ensure!(
        status.success(),
        "systemctl --user start {SYSTEMD_UNIT_NAME} failed"
    );

    Ok(())
}

fn start_with_pid_file() -> Result<()> {
    let server_binary = find_server_binary()?;
    let config_path = expand_path(SERVER_CONFIG_PATH);
    let log_file = expand_path(LOG_FILE_PATH);
    let pid_file = expand_path(PID_FILE_PATH);

    // Ensure log directory exists.
    if let Some(parent) = log_file.parent() {
        fs::create_dir_all(parent)?;
    }

    let log_handle = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .with_context(|| format!("failed to open log file {}", log_file.display()))?;

    let stderr_handle = log_handle
        .try_clone()
        .context("failed to clone log file handle")?;

    let child = Command::new(&server_binary)
        .env("METIS_CONFIG", &config_path)
        .stdout(log_handle)
        .stderr(stderr_handle)
        .spawn()
        .with_context(|| format!("failed to start {}", server_binary.display()))?;

    fs::write(&pid_file, child.id().to_string())
        .with_context(|| format!("failed to write PID file {}", pid_file.display()))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// stop
// ---------------------------------------------------------------------------

fn cmd_stop() -> Result<()> {
    ensure_initialized()?;
    stop_server_platform()
}

#[cfg(target_os = "macos")]
fn stop_server_platform() -> Result<()> {
    stop_with_launchd()
}

#[cfg(target_os = "linux")]
fn stop_server_platform() -> Result<()> {
    stop_with_systemd().or_else(|_| stop_with_pid_file())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn stop_server_platform() -> Result<()> {
    stop_with_pid_file()
}

#[cfg(target_os = "macos")]
fn stop_with_launchd() -> Result<()> {
    let plist_path = expand_path(LAUNCHD_PLIST_PATH);
    if !plist_path.exists() {
        // Fall back to PID file if launchd plist doesn't exist.
        return stop_with_pid_file();
    }

    let _ = Command::new("launchctl")
        .args(["stop", LAUNCHD_LABEL])
        .status();

    let status = Command::new("launchctl")
        .args(["unload"])
        .arg(&plist_path)
        .status()
        .context("failed to run launchctl unload")?;

    if status.success() {
        println!("Server stopped.");
    } else {
        println!("launchctl unload returned non-zero; server may already be stopped.");
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn stop_with_systemd() -> Result<()> {
    let status = Command::new("systemctl")
        .args(["--user", "stop", SYSTEMD_UNIT_NAME])
        .status()
        .context("failed to run systemctl stop")?;

    if status.success() {
        println!("Server stopped.");
        Ok(())
    } else {
        bail!("systemctl --user stop failed; falling back to PID file");
    }
}

fn stop_with_pid_file() -> Result<()> {
    let pid_file = expand_path(PID_FILE_PATH);
    if !pid_file.exists() {
        println!("Server is not running (no PID file found).");
        return Ok(());
    }

    let pid_str = fs::read_to_string(&pid_file)
        .with_context(|| format!("failed to read PID file {}", pid_file.display()))?;
    let pid: i32 = pid_str
        .trim()
        .parse()
        .with_context(|| format!("invalid PID in {}", pid_file.display()))?;

    // Send SIGTERM.
    #[cfg(unix)]
    {
        let ret = unsafe { libc::kill(pid, libc::SIGTERM) };
        if ret == 0 {
            println!("Sent SIGTERM to PID {pid}. Server stopping.");
        } else {
            println!("Failed to send SIGTERM to PID {pid} (process may already be stopped).");
        }
    }

    #[cfg(not(unix))]
    {
        println!("Cannot send signal on this platform. Please stop PID {pid} manually.");
    }

    // Remove the PID file.
    let _ = fs::remove_file(&pid_file);

    Ok(())
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

fn cmd_status() -> Result<()> {
    let server_dir = expand_path(SERVER_DIR);
    if !server_dir.exists() {
        println!("Server is not initialized. Run `metis server init` first.");
        return Ok(());
    }

    if is_server_running()? {
        println!("Server is running.");
        if let Some(pid) = read_pid() {
            println!("  PID: {pid}");
        }
        println!("  URL: {LOCAL_SERVER_URL}");
        println!("  Config: {}", expand_path(SERVER_CONFIG_PATH).display());
        println!("  Logs: {}", expand_path(LOG_FILE_PATH).display());
    } else {
        println!("Server is stopped.");
        println!("  Run `metis server start` to start it.");
    }

    Ok(())
}

fn is_server_running() -> Result<bool> {
    // Try a health check against the server.
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;

    match client.get(format!("{LOCAL_SERVER_URL}/health")).send() {
        Ok(resp) if resp.status().is_success() => Ok(true),
        _ => Ok(false),
    }
}

fn read_pid() -> Option<i32> {
    let pid_file = expand_path(PID_FILE_PATH);
    fs::read_to_string(pid_file)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

// ---------------------------------------------------------------------------
// logs
// ---------------------------------------------------------------------------

fn cmd_logs(lines: usize, follow: bool) -> Result<()> {
    let log_path = expand_path(LOG_FILE_PATH);
    if !log_path.exists() {
        println!("No log file found at {}", log_path.display());
        println!("The server may not have been started yet.");
        return Ok(());
    }

    if follow {
        // Use tail -f for following.
        let status = Command::new("tail")
            .args(["-n", &lines.to_string(), "-f"])
            .arg(&log_path)
            .status()
            .with_context(|| format!("failed to tail {}", log_path.display()))?;

        if !status.success() {
            bail!("tail exited with non-zero status");
        }
    } else {
        // Read the last N lines manually.
        let file = fs::File::open(&log_path)
            .with_context(|| format!("failed to open {}", log_path.display()))?;
        let all_lines: Vec<String> = io::BufReader::new(file)
            .lines()
            .collect::<Result<Vec<_>, _>>()
            .with_context(|| format!("failed to read {}", log_path.display()))?;

        let start = all_lines.len().saturating_sub(lines);
        for line in &all_lines[start..] {
            println!("{line}");
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// restart
// ---------------------------------------------------------------------------

fn cmd_restart() -> Result<()> {
    cmd_stop()?;
    // Brief pause to allow the port to be released.
    thread::sleep(Duration::from_secs(1));
    cmd_start()
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn ensure_initialized() -> Result<()> {
    let config_path = expand_path(SERVER_CONFIG_PATH);
    ensure!(
        config_path.exists(),
        "Server is not initialized. Run `metis server init` first."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_encryption_key_produces_valid_base64() {
        let key = generate_encryption_key();
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&key)
            .expect("key should be valid base64");
        assert_eq!(bytes.len(), 32);
    }

    #[test]
    fn render_server_config_contains_required_fields() {
        let config = render_server_config(
            "dGVzdGtleQ==",
            "ghp_test123",
            Path::new("/tmp/test.db"),
            Path::new("/tmp/auth-token"),
        );
        assert!(config.contains("METIS_SECRET_ENCRYPTION_KEY"));
        assert!(config.contains("ghp_test123"));
        assert!(config.contains("auth_mode: local"));
        assert!(config.contains("storage_backend: sqlite"));
        assert!(config.contains("job_engine: local"));
        assert!(config.contains("/tmp/test.db"));
        assert!(config.contains("auth_token_file: \"/tmp/auth-token\""));
    }

    #[test]
    fn ensure_initialized_fails_when_not_initialized() {
        // Use a temp dir that won't have the config.
        let result = ensure_initialized();
        // This will fail or succeed depending on whether ~/.metis/server/config.yaml exists.
        // In CI this should typically fail.
        if !expand_path(SERVER_CONFIG_PATH).exists() {
            assert!(result.is_err());
        }
    }
}
