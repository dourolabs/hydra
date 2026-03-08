use anyhow::{Context, Result};
use base64::Engine;
use clap::Args;
use metis_common::constants::{LOCAL_AUTH_TOKEN_FILE, METIS_DATA_DIR};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

/// Default server config path inside the data directory.
const SERVER_CONFIG_FILENAME: &str = "server-config.yaml";

/// Local server URL.
const LOCAL_SERVER_URL: &str = "http://localhost:8080";

/// CLI config path (matches the CLI's default).
const CLI_CONFIG_PATH: &str = "~/.local/share/metis/config.toml";

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
    let data_dir = expand_path(METIS_DATA_DIR);
    let config_path = data_dir.join(SERVER_CONFIG_FILENAME);
    let token_path = expand_path(LOCAL_AUTH_TOKEN_FILE);

    // Idempotent: if server config and token file already exist, reuse them.
    if config_path.exists() && token_path.exists() {
        let token = fs::read_to_string(&token_path)
            .context("failed to read existing auth token")?
            .trim()
            .to_string();

        if !token.is_empty() {
            println!("Existing setup detected at {}", data_dir.display());
            println!("Server config: {}", config_path.display());
            println!("Auth token: {}", token_path.display());
            write_cli_config(&token)?;

            if !args.config_only {
                print_start_instructions(&config_path);
            }
            return Ok(());
        }
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
    // create the local user and write the auth token to the well-known file.
    println!("Starting server...");
    // SAFETY: called before spawning additional threads (the server starts
    // inside a single-threaded init flow).
    unsafe {
        std::env::set_var(
            metis_common::constants::ENV_METIS_CONFIG,
            config_path.to_string_lossy().as_ref(),
        );
    }

    // Spawn the server in a background task so we can wait for the token.
    let server_handle = tokio::spawn(async { metis_server::run().await });

    // Wait for the auth token file to appear (the server writes it during
    // setup_local_auth).
    let token = wait_for_token(&token_path).await?;
    write_cli_config(&token)?;

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

async fn wait_for_token(token_path: &Path) -> Result<String> {
    for _ in 0..60 {
        if let Ok(contents) = fs::read_to_string(token_path) {
            let token = contents.trim().to_string();
            if !token.is_empty() {
                return Ok(token);
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    anyhow::bail!(
        "timed out waiting for auth token at '{}'",
        token_path.display()
    )
}

fn write_cli_config(auth_token: &str) -> Result<()> {
    let cli_config_path = expand_path(CLI_CONFIG_PATH);
    if let Some(parent) = cli_config_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create CLI config directory '{}'",
                parent.display()
            )
        })?;
    }

    let config_contents = format!(
        r#"[[servers]]
url = "{LOCAL_SERVER_URL}"
auth_token = "{auth_token}"
default = true
"#
    );
    fs::write(&cli_config_path, config_contents).with_context(|| {
        format!(
            "failed to write CLI config to '{}'",
            cli_config_path.display()
        )
    })?;
    println!("CLI config written to {}", cli_config_path.display());
    Ok(())
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

fn expand_path(path: &str) -> PathBuf {
    if path.starts_with('~') {
        PathBuf::from(shellexpand::tilde(path).into_owned())
    } else {
        PathBuf::from(path)
    }
}
