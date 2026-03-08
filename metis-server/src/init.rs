use anyhow::{Context, Result};
use base64::Engine;
use clap::Args;
use metis_common::constants::{DEFAULT_CLI_CONFIG_PATH, DEFAULT_DATA_DIR};
use std::{fs, path::Path};
use uuid::Uuid;

/// Default server config path inside the data directory.
const SERVER_CONFIG_FILENAME: &str = "server-config.yaml";

/// Local server URL.
const LOCAL_SERVER_URL: &str = "http://localhost:8080";

#[derive(Args)]
pub struct InitArgs {
    /// GitHub personal access token for GitHub API access.
    #[arg(long = "github-token", env = "GITHUB_TOKEN")]
    github_token: String,

    /// Only generate config files, do not start the server.
    #[arg(long)]
    config_only: bool,
}

pub async fn run(args: &InitArgs) -> Result<()> {
    let data_dir = metis_server::config::expand_path(DEFAULT_DATA_DIR);
    let config_path = data_dir.join(SERVER_CONFIG_FILENAME);
    let cli_config_path = metis_server::config::expand_path(DEFAULT_CLI_CONFIG_PATH);

    // Idempotent: if server config already exists and CLI config has a token, reuse them.
    if config_path.exists() && has_cli_auth_token(&cli_config_path) {
        println!("Existing setup detected at {}", data_dir.display());
        println!("Server config: {}", config_path.display());
        println!("CLI config: {}", cli_config_path.display());

        if !args.config_only {
            print_start_instructions(&config_path);
        }
        return Ok(());
    }

    // Create data directory.
    fs::create_dir_all(&data_dir)
        .with_context(|| format!("failed to create data directory '{}'", data_dir.display()))?;

    // Generate a 32-byte encryption key from two UUIDs (16 bytes each).
    let mut key_bytes = [0u8; 32];
    key_bytes[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    key_bytes[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    let encryption_key = base64::engine::general_purpose::STANDARD.encode(key_bytes);

    // Build SQLite path.
    let db_path = data_dir.join("metis.db");
    let db_path_str = db_path.to_string_lossy();

    // Write server config.
    let config_contents = build_server_config(&encryption_key, &args.github_token, &db_path_str);
    fs::write(&config_path, &config_contents).with_context(|| {
        format!(
            "failed to write server config to '{}'",
            config_path.display()
        )
    })?;
    println!("Server config written to {}", config_path.display());

    if args.config_only {
        println!();
        print_start_instructions(&config_path);
        println!("After the server starts, run `metis-server init` again (without --config-only)");
        println!("to configure the CLI with the generated auth token.");
        return Ok(());
    }

    // Start the server in-process: load config, run startup, which will
    // create the local user and write the auth token to the CLI config.
    println!("Starting server...");

    // Spawn the server in a background task so we can wait for the CLI config.
    let server_config_path = config_path.clone();
    let server_handle =
        tokio::spawn(
            async move { metis_server::run_with_config_path(Some(server_config_path)).await },
        );

    // Wait for the CLI config to be written with an auth token (the server
    // writes it during setup_local_auth).
    wait_for_cli_config(&cli_config_path).await?;

    println!();
    println!("Metis is ready!");
    println!("  Server: {LOCAL_SERVER_URL}");
    println!("  CLI configured to connect to local server.");
    println!();
    println!("Try: metis issues list");

    // Keep running the server until interrupted.
    server_handle
        .await
        .context("server task panicked")?
        .context("server exited with error")?;

    Ok(())
}

/// Check whether the CLI config file contains an auth token for the local server.
fn has_cli_auth_token(cli_config_path: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(cli_config_path) else {
        return false;
    };
    let Ok(parsed) = toml::from_str::<toml::Value>(&contents) else {
        return false;
    };
    let Some(servers) = parsed.get("servers").and_then(|v| v.as_array()) else {
        return false;
    };
    servers.iter().any(|server| {
        let url_matches = server
            .get("url")
            .and_then(|v| v.as_str())
            .is_some_and(|url| url.trim_end_matches('/') == LOCAL_SERVER_URL);
        let has_token = server
            .get("auth_token")
            .and_then(|v| v.as_str())
            .is_some_and(|t| !t.is_empty());
        url_matches && has_token
    })
}

async fn wait_for_cli_config(cli_config_path: &Path) -> Result<()> {
    for _ in 0..60 {
        if has_cli_auth_token(cli_config_path) {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    anyhow::bail!(
        "timed out waiting for CLI config to be written at '{}'",
        cli_config_path.display()
    )
}

fn build_server_config(encryption_key: &str, github_token: &str, db_path: &str) -> String {
    format!(
        r#"metis:
  namespace: "default"
  METIS_SECRET_ENCRYPTION_KEY: "{encryption_key}"
  allowed_orgs: []

storage_backend: "sqlite"
sqlite_path: "{db_path}"

job_engine: "local"

auth_mode: "local"
github_token: "{github_token}"

job:
  default_image: "metis-worker:latest"
"#
    )
}

fn print_start_instructions(config_path: &Path) {
    println!("To start the server, run:");
    println!("  METIS_CONFIG={} metis-server", config_path.display());
}
