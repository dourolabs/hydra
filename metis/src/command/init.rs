use anyhow::{bail, Context, Result};
use base64::Engine;
use clap::Args;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use crate::config;

/// Default server URL for a locally initialized Metis server.
const LOCAL_SERVER_URL: &str = "http://localhost:8080";

/// Directory for local Metis data files (server config, database, etc.).
const DATA_DIR: &str = "~/.local/share/metis";

/// Path to the generated server config file.
const SERVER_CONFIG_PATH: &str = "~/.local/share/metis/server-config.yaml";

#[derive(Debug, Args)]
pub struct InitArgs {
    /// GitHub personal access token for GitHub API access (PR sync, etc.).
    /// Required unless --config-only is used with an existing config.
    #[arg(long = "github-token", env = "GITHUB_TOKEN", value_name = "TOKEN")]
    pub github_token: Option<String>,

    /// Only generate config files; do not start the server.
    #[arg(long = "config-only")]
    pub config_only: bool,

    /// Start the server in the background as a daemon process.
    #[arg(long = "daemon")]
    pub daemon: bool,
}

pub async fn run(args: &InitArgs, cli_config_path: &Path) -> Result<()> {
    let data_dir = config::expand_path(DATA_DIR);
    let server_config_path = config::expand_path(SERVER_CONFIG_PATH);
    let token_file_path = config::expand_path(metis_common::constants::LOCAL_AUTH_TOKEN_FILE);

    // Idempotent: detect existing setup.
    if server_config_path.exists() && token_file_path.exists() {
        let existing_token = fs::read_to_string(&token_file_path)
            .context("failed to read existing local auth token")?;
        if !existing_token.trim().is_empty() {
            eprintln!("Metis is already initialized.");
            eprintln!("  Server config: {}", server_config_path.display());
            eprintln!("  Auth token:    {}", token_file_path.display());

            // Ensure CLI config points at the local server.
            ensure_cli_config(cli_config_path, existing_token.trim())?;

            if !args.config_only {
                start_server(&server_config_path, args.daemon).await?;
            }
            return Ok(());
        }
    }

    // Require a GitHub token for fresh initialization.
    let github_token = args
        .github_token
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .context(
            "a GitHub personal access token is required for initialization; \
             pass --github-token or set the GITHUB_TOKEN environment variable",
        )?;

    // Generate encryption key (32 random bytes, base64-encoded).
    let encryption_key = generate_encryption_key();

    // Create data directory.
    fs::create_dir_all(&data_dir)
        .with_context(|| format!("failed to create data directory '{}'", data_dir.display()))?;

    // Build the SQLite database URL.
    let db_path = data_dir.join("metis.db");
    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());

    // Write server config.
    let server_config = build_server_config(&encryption_key, github_token, &db_url);
    fs::write(&server_config_path, &server_config).with_context(|| {
        format!(
            "failed to write server config to '{}'",
            server_config_path.display()
        )
    })?;
    eprintln!("Generated server config: {}", server_config_path.display());

    if args.config_only {
        eprintln!(
            "Config generated. Start the server with:\n  \
             METIS_CONFIG={} cargo run -p metis-server",
            server_config_path.display()
        );
        return Ok(());
    }

    // Start the server and wait for the auth token file to appear.
    start_server(&server_config_path, args.daemon).await?;

    // Read the auth token written by the server.
    let auth_token = wait_for_token_file(&token_file_path).await?;

    // Configure CLI to use the local server.
    ensure_cli_config(cli_config_path, &auth_token)?;

    eprintln!("Metis initialized successfully.");
    eprintln!("  Server:  {LOCAL_SERVER_URL}");
    eprintln!("  Config:  {}", server_config_path.display());
    eprintln!(
        "  CLI config: {}",
        config::expand_path(cli_config_path).display()
    );

    Ok(())
}

fn generate_encryption_key() -> String {
    let mut key = [0u8; 32];
    getrandom::getrandom(&mut key).expect("failed to generate random key");
    base64::engine::general_purpose::STANDARD.encode(key)
}

fn build_server_config(encryption_key: &str, github_token: &str, db_url: &str) -> String {
    format!(
        r#"metis:
  namespace: "default"
  METIS_SECRET_ENCRYPTION_KEY: "{encryption_key}"
  allowed_orgs: []

auth_mode: local
github_token: "{github_token}"

job:
  engine: docker
  default_image: "metis-worker:latest"

database:
  backend: sqlite
  url: "{db_url}"
"#
    )
}

async fn start_server(config_path: &Path, daemon: bool) -> Result<()> {
    let metis_server_bin = find_server_binary()?;

    eprintln!("Starting metis-server...");

    if daemon {
        let mut cmd = std::process::Command::new(&metis_server_bin);
        cmd.env(metis_common::constants::ENV_METIS_CONFIG, config_path);
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        // Detach the child so it survives the parent process.
        #[cfg(unix)]
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }

        let child = cmd.spawn().context("failed to start metis-server")?;
        eprintln!("Server started in background (pid: {})", child.id());
    } else {
        // Foreground: spawn server as a task and wait for it to become healthy.
        let config_path = config_path.to_path_buf();
        tokio::spawn(async move {
            let mut cmd = tokio::process::Command::new(&metis_server_bin);
            cmd.env(metis_common::constants::ENV_METIS_CONFIG, &config_path);
            match cmd.status().await {
                Ok(status) if !status.success() => {
                    eprintln!("metis-server exited with status: {status}");
                }
                Err(err) => {
                    eprintln!("metis-server failed: {err}");
                }
                _ => {}
            }
        });
    }

    // Wait for the server to become healthy.
    wait_for_health().await?;
    eprintln!("Server is healthy at {LOCAL_SERVER_URL}");

    Ok(())
}

fn find_server_binary() -> Result<PathBuf> {
    // Try to find the metis-server binary next to the current executable.
    if let Ok(current_exe) = std::env::current_exe() {
        let dir = current_exe.parent().unwrap_or(Path::new("."));
        let candidate = dir.join("metis-server");
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    // Fall back to searching PATH.
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("metis-server");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    bail!("could not find metis-server binary; build it with: cargo build -p metis-server")
}

async fn wait_for_health() -> Result<()> {
    let client = reqwest::Client::new();
    let url = format!("{LOCAL_SERVER_URL}/health");

    for i in 0..30 {
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            _ => {}
        }
    }
    bail!("server did not become healthy within 15 seconds")
}

async fn wait_for_token_file(path: &Path) -> Result<String> {
    for i in 0..30 {
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        if let Ok(contents) = fs::read_to_string(path) {
            let token = contents.trim().to_string();
            if !token.is_empty() {
                return Ok(token);
            }
        }
    }
    bail!(
        "local auth token file did not appear at '{}' within 15 seconds",
        path.display()
    )
}

fn ensure_cli_config(cli_config_path: &Path, auth_token: &str) -> Result<()> {
    config::store_auth_token(cli_config_path, LOCAL_SERVER_URL, auth_token)
        .context("failed to update CLI config with local auth token")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_encryption_key_is_valid_base64_32_bytes() {
        let key = generate_encryption_key();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&key)
            .expect("key should be valid base64");
        assert_eq!(bytes.len(), 32);
    }

    #[test]
    fn build_server_config_produces_valid_yaml() {
        let config = build_server_config(
            "dGVzdGtleXRlc3RrZXl0ZXN0a2V5dGVzdGtleXk=",
            "ghp_test",
            "sqlite:test.db",
        );
        // Should parse as valid YAML.
        let value: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(&config).expect("config should be valid YAML");
        assert_eq!(value["auth_mode"].as_str(), Some("local"));
        assert_eq!(value["database"]["backend"].as_str(), Some("sqlite"));
        assert_eq!(value["job"]["engine"].as_str(), Some("docker"));
    }

    #[test]
    fn ensure_cli_config_creates_config_file() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let config_path = temp_dir.path().join("config.toml");

        ensure_cli_config(&config_path, "local:test-token").expect("ensure_cli_config");

        let config = crate::config::AppConfig::load(&config_path).expect("load config");
        let token = config
            .auth_token_for_url(LOCAL_SERVER_URL)
            .expect("lookup token");
        assert_eq!(token, Some("local:test-token"));
    }
}
