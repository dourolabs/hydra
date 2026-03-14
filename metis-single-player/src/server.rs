use std::{
    fs,
    io::{self, BufRead, Write},
    path::Path,
    process::Command,
    thread,
    time::Duration,
};

use anyhow::{bail, ensure, Context, Result};
use clap::Subcommand;

use metis::client::MetisClient;
use metis::config::{self, expand_path};
use metis::constants::DEFAULT_CONFIG_FILE;
use metis_common::api::v1::agents::UpsertAgentRequest;

/// Directory layout under ~/.metis/
const SERVER_DIR: &str = "~/.metis/server";
const SERVER_CONFIG_PATH: &str = "~/.metis/server/config.yaml";
const AUTH_TOKEN_PATH: &str = "~/.metis/server/auth-token";
const PID_FILE_PATH: &str = "~/.metis/server/metis-server.pid";
const LOG_DIR: &str = "~/.metis/server/logs";
const LOG_FILE_PATH: &str = "~/.metis/server/logs/metis-server.log";
const JOB_LOG_DIR: &str = "~/.metis/server/job-logs";
const SERVER_DB_PATH: &str = "~/.metis/server/metis.db";

const LOCAL_SERVER_URL: &str = "http://127.0.0.1:8080";

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

    // Prompt for username.
    let username = prompt_username()?;

    // Prompt for job engine choice.
    let job_engine = prompt_job_engine()?;

    // Prompt for model choice (Codex vs Claude).
    let (provider, default_model) = prompt_model_choice()?;

    // Prompt for the appropriate API key(s).
    let api_keys = prompt_api_key(provider)?;

    // Prompt for GitHub PAT.
    let github_pat = prompt_github_pat()?;

    // Generate a random 32-byte encryption key (base64-encoded).
    let encryption_key = generate_encryption_key();

    // Create directory structure.
    let log_dir = expand_path(LOG_DIR);
    let job_log_dir = expand_path(JOB_LOG_DIR);
    fs::create_dir_all(&server_dir)
        .with_context(|| format!("failed to create {}", server_dir.display()))?;
    fs::create_dir_all(&log_dir)
        .with_context(|| format!("failed to create {}", log_dir.display()))?;
    fs::create_dir_all(&job_log_dir)
        .with_context(|| format!("failed to create {}", job_log_dir.display()))?;

    // Write server config.
    let db_path = expand_path(SERVER_DB_PATH);
    let auth_token_path_expanded = expand_path(AUTH_TOKEN_PATH);
    let job_log_dir_str = if job_engine == "local" {
        Some(job_log_dir.display().to_string())
    } else {
        None
    };
    let config_content = render_server_config(
        &encryption_key,
        &github_pat,
        &db_path,
        &auth_token_path_expanded,
        &job_engine,
        Some(&default_model),
        &api_keys,
        username.as_deref(),
        job_log_dir_str.as_deref(),
    );
    fs::write(&config_path, &config_content)
        .with_context(|| format!("failed to write config to {}", config_path.display()))?;

    println!("Server config written to {}", config_path.display());

    // Start the server in-process so it creates the local user and auth token.
    println!("Starting server...");
    start_server_in_process()?;

    // Poll /health to verify the server is actually running before continuing.
    wait_for_server_healthy()?;

    // Wait for the auth token file to appear (the server writes it on startup).
    let token_path = expand_path(AUTH_TOKEN_PATH);
    let auth_token = wait_for_auth_token(&token_path)?;

    let cli_config_path = expand_path(Path::new(DEFAULT_CONFIG_FILE));
    config::store_auth_token(&cli_config_path, LOCAL_SERVER_URL, &auth_token)?;
    let mut cli_config = config::AppConfig::load(&cli_config_path)?;
    cli_config.set_default_server(LOCAL_SERVER_URL)?;
    cli_config.write_to(&cli_config_path)?;
    println!("CLI configured with auth token for {LOCAL_SERVER_URL}");

    // Auto-populate default agents and their prompts.
    create_default_agents(&auth_token)?;

    // Upload default playbooks to the document store.
    upload_default_playbooks(&auth_token)?;

    let engine_label = if job_engine == "docker" {
        "Docker"
    } else {
        "Local"
    };
    println!();
    println!("Metis is running! Dashboard: {LOCAL_SERVER_URL}");
    println!("Job engine: {engine_label}");
    println!();
    println!("Next steps:");
    println!("  metis issues list              # list issues");
    println!("  metis server status             # check server status");
    println!("  metis server logs --follow      # watch server logs");
    println!("  metis server stop               # stop the server");

    Ok(())
}

// Embedded agent prompts (compiled into the binary).
const SWE_PROMPT: &str = include_str!("../../prompts/agents/swe.md");
const PM_PROMPT: &str = include_str!("../../prompts/agents/pm.md");
const REVIEWER_PROMPT: &str = include_str!("../../prompts/agents/reviewer.md");

// Embedded playbook content (compiled into the binary).
const PLAYBOOK_ADD_NEW_REPO: &str = include_str!("../../prompts/playbooks/add-new-repo.md");
const PLAYBOOK_DESIGN_REVIEW: &str = include_str!("../../prompts/playbooks/design-review.md");

/// Create the default agents (swe, pm, reviewer) and upload their prompts
/// to the running server via the MetisClient.
fn create_default_agents(auth_token: &str) -> Result<()> {
    let client = MetisClient::new(LOCAL_SERVER_URL, auth_token)?;

    let agents: &[(&str, &str, bool)] = &[
        ("swe", SWE_PROMPT, false),
        ("pm", PM_PROMPT, true),
        ("reviewer", REVIEWER_PROMPT, false),
    ];

    // Server commands run before the tokio runtime is created (due to fork),
    // so we create a small runtime to drive the async MetisClient calls.
    let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;

    for &(name, prompt, is_assignment_agent) in agents {
        let mut request = UpsertAgentRequest::new(name, prompt, 3, i32::MAX);
        request.is_assignment_agent = is_assignment_agent;

        rt.block_on(client.create_agent(&request))
            .with_context(|| format!("failed to create agent '{name}'"))?;
        println!("Created agent: {name}");
    }

    Ok(())
}

/// Upload default playbooks to the server's document store.
fn upload_default_playbooks(auth_token: &str) -> Result<()> {
    use metis_common::api::v1::documents::{Document, UpsertDocumentRequest};

    let playbooks = [
        (
            "Add new repo to metis",
            PLAYBOOK_ADD_NEW_REPO,
            "playbooks/add-new-repo.md",
        ),
        (
            "Design Document Review",
            PLAYBOOK_DESIGN_REVIEW,
            "playbooks/design-review.md",
        ),
    ];

    let client = MetisClient::new(LOCAL_SERVER_URL, auth_token)?;
    let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;

    for (title, body, path) in playbooks {
        let document = Document::new(
            title.to_string(),
            body.to_string(),
            Some(path.to_string()),
            None,
            false,
        )
        .with_context(|| format!("invalid document path {path}"))?;
        let request = UpsertDocumentRequest::new(document);

        rt.block_on(client.create_document(&request))
            .with_context(|| format!("failed to upload playbook {path}"))?;

        println!("Uploaded playbook: {path}");
    }

    Ok(())
}

/// Check whether Docker is available by running `docker info`.
fn is_docker_available() -> bool {
    Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Prompt the user to choose between Docker and Local job engines.
/// Returns `"docker"` for Docker or `"local"` for Local.
fn prompt_job_engine() -> Result<String> {
    let docker_available = is_docker_available();

    let docker_status = if docker_available {
        "available"
    } else {
        "not detected"
    };
    let default_choice = if docker_available { "1" } else { "2" };

    eprintln!();
    eprintln!("Select job engine:");
    eprintln!("  1) Docker (recommended) - runs jobs in isolated containers [{docker_status}]");
    eprintln!("  2) Local - runs jobs directly on this computer (less isolation)");
    eprint!("Choice [{default_choice}]: ");
    io::stderr().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    let choice = if input.is_empty() {
        default_choice
    } else {
        input
    };

    match choice {
        "1" => {
            if !docker_available {
                eprintln!();
                eprintln!(
                    "Warning: Docker was not detected on this system. \
                     Please install Docker (https://docs.docker.com/get-docker/) \
                     and ensure it is running before starting jobs."
                );
                eprintln!();
            }
            Ok("docker".to_string())
        }
        "2" => Ok("local".to_string()),
        _ => {
            eprintln!("Invalid choice '{choice}', using default ({default_choice}).");
            if docker_available {
                Ok("docker".to_string())
            } else {
                Ok("local".to_string())
            }
        }
    }
}

fn prompt_github_pat() -> Result<String> {
    eprint!("Enter your GitHub Personal Access Token (PAT): ");
    io::stderr().flush()?;
    let token = rpassword::prompt_password_stdout("").context("failed to read GitHub PAT")?;
    let token = token.trim().to_string();
    ensure!(!token.is_empty(), "GitHub PAT is required");
    Ok(token)
}

/// Prompt the user for their desired username.
/// Returns `None` when the user accepts the default ("local").
fn prompt_username() -> Result<Option<String>> {
    eprint!("Enter your username [local]: ");
    io::stderr().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if input.is_empty() || input == "local" {
        return Ok(None);
    }

    ensure!(
        !input.contains(char::is_whitespace),
        "Username must not contain whitespace"
    );
    ensure!(
        input
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_'),
        "Username must contain only alphanumeric characters, hyphens, or underscores"
    );

    Ok(Some(input.to_string()))
}

/// Which AI provider the user selected during init.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelProvider {
    Codex,
    Claude,
}

/// Prompt the user to choose between Codex and Claude as their default model.
/// Returns the provider enum and the model string to store in config.
fn prompt_model_choice() -> Result<(ModelProvider, String)> {
    eprintln!();
    eprintln!("Select default model:");
    eprintln!("  1) Codex (OpenAI gpt-4o)");
    eprintln!("  2) Claude (Anthropic opus)");
    eprint!("Choice [2]: ");
    io::stderr().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    let choice = if input.is_empty() { "2" } else { input };

    match choice {
        "1" => Ok((ModelProvider::Codex, "gpt-4o".to_string())),
        "2" => Ok((ModelProvider::Claude, "opus".to_string())),
        _ => {
            eprintln!("Invalid choice '{choice}', defaulting to Claude.");
            Ok((ModelProvider::Claude, "opus".to_string()))
        }
    }
}

/// API keys collected during init.
#[derive(Debug, Default)]
struct ApiKeys {
    openai_api_key: Option<String>,
    anthropic_api_key: Option<String>,
    claude_code_oauth_token: Option<String>,
}

/// Prompt for the appropriate API key(s) based on the selected provider.
fn prompt_api_key(provider: ModelProvider) -> Result<ApiKeys> {
    let mut keys = ApiKeys::default();
    match provider {
        ModelProvider::Codex => {
            eprintln!();
            eprint!("Enter your OpenAI API key (OPENAI_API_KEY): ");
            io::stderr().flush()?;
            let key =
                rpassword::prompt_password_stdout("").context("failed to read OpenAI API key")?;
            let key = key.trim().to_string();
            if !key.is_empty() {
                keys.openai_api_key = Some(key);
            }
        }
        ModelProvider::Claude => {
            eprintln!();
            eprintln!("Select Claude authentication method:");
            eprintln!("  1) API key (ANTHROPIC_API_KEY)");
            eprintln!("  2) OAuth token (CLAUDE_CODE_OAUTH_TOKEN)");
            eprint!("Choice [1]: ");
            io::stderr().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let input = input.trim();
            let choice = if input.is_empty() { "1" } else { input };

            match choice {
                "2" => {
                    eprint!("Enter your Claude OAuth token (CLAUDE_CODE_OAUTH_TOKEN): ");
                    io::stderr().flush()?;
                    let key = rpassword::prompt_password_stdout("")
                        .context("failed to read Claude OAuth token")?;
                    let key = key.trim().to_string();
                    if !key.is_empty() {
                        keys.claude_code_oauth_token = Some(key);
                    }
                }
                _ => {
                    eprint!("Enter your Anthropic API key (ANTHROPIC_API_KEY): ");
                    io::stderr().flush()?;
                    let key = rpassword::prompt_password_stdout("")
                        .context("failed to read Anthropic API key")?;
                    let key = key.trim().to_string();
                    if !key.is_empty() {
                        keys.anthropic_api_key = Some(key);
                    }
                }
            }
        }
    }
    Ok(keys)
}

fn generate_encryption_key() -> String {
    let mut key = [0u8; 32];
    getrandom::getrandom(&mut key).expect("failed to generate random bytes from OS CSPRNG");
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(key)
}

#[allow(clippy::too_many_arguments)]
fn render_server_config(
    encryption_key: &str,
    github_pat: &str,
    db_path: &Path,
    auth_token_path: &Path,
    job_engine: &str,
    default_model: Option<&str>,
    api_keys: &ApiKeys,
    username: Option<&str>,
    job_log_dir: Option<&str>,
) -> String {
    use metis_server::config::{
        AppConfig, AuthConfig, BackgroundSection, BuildCacheSection, JobEngineConfig, JobSection,
        MetisSection, StorageConfig,
    };

    let job_engine_config = match job_engine {
        "local" => JobEngineConfig::Local {
            log_dir: job_log_dir.map(str::to_string),
        },
        _ => JobEngineConfig::Docker,
    };

    let config = AppConfig {
        metis: MetisSection {
            namespace: "default".to_string(),
            server_hostname: "127.0.0.1:8080".to_string(),
            secret_encryption_key: encryption_key.to_string(),
            allowed_orgs: Vec::new(),
            openai_api_key: api_keys.openai_api_key.clone(),
            anthropic_api_key: api_keys.anthropic_api_key.clone(),
            claude_code_oauth_token: api_keys.claude_code_oauth_token.clone(),
        },
        job: JobSection {
            default_image: "ubuntu:24.04".to_string(),
            default_model: default_model.map(str::to_string),
            cpu_limit: "500m".to_string(),
            memory_limit: "1Gi".to_string(),
            cpu_request: "500m".to_string(),
            memory_request: "1Gi".to_string(),
        },
        storage: StorageConfig::Sqlite {
            sqlite_path: db_path.display().to_string(),
        },
        job_engine: job_engine_config,
        auth: AuthConfig::Local {
            github_token: github_pat.to_string(),
            username: username.map(str::to_string),
            auth_token_file: Some(auth_token_path.to_path_buf()),
        },
        background: BackgroundSection::default(),
        build_cache: BuildCacheSection::default(),
        policies: None,
    };

    let yaml = serde_yaml_ng::to_string(&config).expect("failed to serialize server init config");
    format!("# Metis server configuration (auto-generated by `metis server init`)\n{yaml}")
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

fn wait_for_server_healthy() -> Result<()> {
    let max_wait = Duration::from_secs(30);
    let poll_interval = Duration::from_millis(500);
    let start = std::time::Instant::now();

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;

    while start.elapsed() < max_wait {
        match client.get(format!("{LOCAL_SERVER_URL}/health")).send() {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            _ => {}
        }
        thread::sleep(poll_interval);
    }

    bail!(
        "Server failed to start — /health did not respond within 30 seconds. \
         Check logs with `metis server logs`."
    );
}

// ---------------------------------------------------------------------------
// start
// ---------------------------------------------------------------------------

fn cmd_start() -> Result<()> {
    ensure_initialized()?;

    if is_server_running()? {
        println!("Server is already running.");
        print_running_status();
        return Ok(());
    }

    start_server_in_process()?;
    println!("Server started.");
    print_running_status();
    Ok(())
}

/// Start the server in-process by forking the current process and running
/// `metis_server::run()` in the child. The child's PID is written to the
/// PID file so that `metis server stop` can find it.
fn start_server_in_process() -> Result<()> {
    let config_path = expand_path(SERVER_CONFIG_PATH);
    let log_file = expand_path(LOG_FILE_PATH);
    let pid_file = expand_path(PID_FILE_PATH);

    // Ensure log directory exists.
    if let Some(parent) = log_file.parent() {
        fs::create_dir_all(parent)?;
    }

    // Set the server config env var so `metis_server::run()` can find it.
    // This is safe because we run before the tokio runtime is created
    // (no other threads exist yet).
    unsafe {
        std::env::set_var("METIS_CONFIG", &config_path);
    }

    // Fork the process. The child runs the server; the parent records
    // the child PID and returns.
    #[cfg(unix)]
    {
        use std::os::unix::io::IntoRawFd;

        // Open the log file for the child's stdout/stderr.
        let log_handle = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)
            .with_context(|| format!("failed to open log file {}", log_file.display()))?;
        let log_fd = log_handle.into_raw_fd();

        // Safety: fork() is called before the tokio runtime is created.
        // main() handles the server subcommand synchronously before calling
        // tokio::runtime::Runtime::new(), so no worker threads exist yet.
        let fork_result = unsafe { libc::fork() };
        match fork_result {
            -1 => bail!("fork() failed: {}", io::Error::last_os_error()),
            0 => {
                // Child process — become a new session leader (detach from terminal).
                unsafe {
                    libc::setsid();
                }

                // Redirect stdout and stderr to the log file.
                unsafe {
                    libc::dup2(log_fd, libc::STDOUT_FILENO);
                    libc::dup2(log_fd, libc::STDERR_FILENO);
                    libc::close(log_fd);
                }

                // Redirect stdin to /dev/null so the background server doesn't
                // consume terminal input meant for the user's shell.
                unsafe {
                    let dev_null = libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY);
                    if dev_null >= 0 {
                        libc::dup2(dev_null, libc::STDIN_FILENO);
                        libc::close(dev_null);
                    }
                }

                // Build a new tokio runtime and run the server with BFF.
                let rt = tokio::runtime::Runtime::new()
                    .expect("failed to create tokio runtime for in-process server");
                let result = rt.block_on(run_server_with_bff());
                if let Err(e) = result {
                    eprintln!("metis-server exited with error: {e:#}");
                    std::process::exit(1);
                }
                std::process::exit(0);
            }
            child_pid => {
                // Parent process — record the child PID.
                unsafe {
                    libc::close(log_fd);
                }
                fs::write(&pid_file, child_pid.to_string())
                    .with_context(|| format!("failed to write PID file {}", pid_file.display()))?;
            }
        }
    }

    #[cfg(not(unix))]
    {
        bail!(
            "In-process server startup via fork is only supported on Unix. \
             Please run `metis-server` as a separate process."
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// run server with BFF
// ---------------------------------------------------------------------------

/// Start the metis server with the BFF layer (auth routes, API proxy,
/// embedded frontend, auto-login). This replaces the plain `metis_server::run()`
/// to add the single-player-specific features.
async fn run_server_with_bff() -> Result<()> {
    tracing_subscriber::fmt::init();

    let config_path = metis_server::config_path();
    let app_config = metis_server::config::AppConfig::load(&config_path)?;

    // Resolve the auth token file path before moving app_config into build_app_state.
    let auth_token_path = app_config.auth.auth_token_file().map(|p| p.to_path_buf());

    // Build app state first — this calls setup_local_auth() which writes the auth token file.
    let state = metis_server::build_app_state(app_config).await?;

    // Now read the auth token for auto-login (the file was created by build_app_state).
    let auto_login_token = match auth_token_path {
        Some(path) => {
            let token = fs::read_to_string(&path)
                .with_context(|| format!("failed to read auth token from {}", path.display()))?;
            let token = token.trim().to_string();
            if token.is_empty() {
                bail!(
                    "auth_token_file {} is empty after server init; auto-login unavailable",
                    path.display()
                );
            }
            token
        }
        None => bail!("auth_token_file is required for single-player mode auto-login"),
    };

    // Build the internal metis-server router with state applied.
    let inner_app = metis_server::build_router(&state).with_state(state.clone());

    let bff_app = crate::bff::build_bff_router(inner_app, auto_login_token);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;

    metis_server::run_with_state(state, listener, bff_app).await
}

// ---------------------------------------------------------------------------
// stop
// ---------------------------------------------------------------------------

fn cmd_stop() -> Result<()> {
    ensure_initialized()?;
    stop_server()
}

fn stop_server() -> Result<()> {
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
        print_running_status();
    } else {
        println!("Server is stopped.");
        println!("  Run `metis server start` to start it.");
    }

    Ok(())
}

/// Print the status details (PID, URL, config path, logs path) for a running server.
fn print_running_status() {
    if let Some(pid) = read_pid() {
        println!("  PID: {pid}");
    }
    println!("  URL: {LOCAL_SERVER_URL}");
    println!("  Config: {}", expand_path(SERVER_CONFIG_PATH).display());
    println!("  Logs: {}", expand_path(LOG_FILE_PATH).display());
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
        // Use a valid 32-byte base64 key so the round-trip succeeds validation.
        use base64::Engine;
        let encryption_key = base64::engine::general_purpose::STANDARD.encode([42u8; 32]);

        let config = render_server_config(
            &encryption_key,
            "ghp_test123",
            Path::new("/tmp/test.db"),
            Path::new("/tmp/auth-token"),
            "docker",
            None,
            &ApiKeys::default(),
            None,
            None,
        );

        // Verify the generated YAML contains all expected fields.
        assert!(config.contains("METIS_SECRET_ENCRYPTION_KEY"));
        assert!(config.contains("ghp_test123"));
        assert!(config.contains("auth_mode: local"));
        assert!(config.contains("storage_backend: sqlite"));
        assert!(config.contains("job_engine: docker"));
        assert!(config.contains("/tmp/test.db"));
        assert!(config.contains("/tmp/auth-token"));
        assert!(config.contains("default_image:"));
        assert!(config.contains("# Metis server configuration"));
        assert!(config.contains("server_hostname: 127.0.0.1:8080"));
        // Username should not appear when None.
        assert!(!config.contains("username:"));

        // Round-trip: deserialize the generated YAML back into AppConfig.
        use metis_server::config::AppConfig;
        let app_config: AppConfig = serde_yaml_ng::from_str(&config)
            .expect("generated config should deserialize into AppConfig");

        assert_eq!(app_config.metis.secret_encryption_key, encryption_key);
        assert!(app_config.auth.is_local());
        assert_eq!(app_config.auth.github_token(), Some("ghp_test123"));
        assert_eq!(
            app_config.auth.auth_token_file(),
            Some(Path::new("/tmp/auth-token"))
        );
        assert_eq!(app_config.job.default_image, "ubuntu:24.04");
        assert!(app_config.job.default_model.is_none());
        assert!(app_config.metis.openai_api_key.is_none());
        assert!(app_config.metis.anthropic_api_key.is_none());
        assert!(app_config.metis.claude_code_oauth_token.is_none());
        assert!(matches!(
            app_config.storage,
            metis_server::config::StorageConfig::Sqlite { .. }
        ));
        assert!(matches!(
            app_config.job_engine,
            metis_server::config::JobEngineConfig::Docker
        ));
    }

    #[test]
    fn render_server_config_local_engine() {
        use base64::Engine;
        let encryption_key = base64::engine::general_purpose::STANDARD.encode([42u8; 32]);

        let config = render_server_config(
            &encryption_key,
            "ghp_test123",
            Path::new("/tmp/test.db"),
            Path::new("/tmp/auth-token"),
            "local",
            None,
            &ApiKeys::default(),
            None,
            Some("/custom/log/dir"),
        );

        assert!(config.contains("job_engine: local"));
        assert!(config.contains("server_hostname: 127.0.0.1:8080"));
        assert!(config.contains("/custom/log/dir"));

        use metis_server::config::AppConfig;
        let app_config: AppConfig = serde_yaml_ng::from_str(&config)
            .expect("generated config should deserialize into AppConfig");

        match &app_config.job_engine {
            metis_server::config::JobEngineConfig::Local { log_dir } => {
                assert_eq!(*log_dir, Some("/custom/log/dir".to_string()));
            }
            other => panic!("expected JobEngineConfig::Local, got {other:?}"),
        }
        assert_eq!(app_config.metis.server_hostname, "127.0.0.1:8080");
    }

    #[test]
    fn render_server_config_with_codex_model_and_openai_key() {
        use base64::Engine;
        let encryption_key = base64::engine::general_purpose::STANDARD.encode([42u8; 32]);

        let keys = ApiKeys {
            openai_api_key: Some("sk-test-openai-key".to_string()),
            ..Default::default()
        };

        let config = render_server_config(
            &encryption_key,
            "ghp_test123",
            Path::new("/tmp/test.db"),
            Path::new("/tmp/auth-token"),
            "docker",
            Some("gpt-4o"),
            &keys,
            None,
            None,
        );

        assert!(config.contains("default_model: gpt-4o"));
        assert!(config.contains("OPENAI_API_KEY: sk-test-openai-key"));
        // Should not contain Claude keys.
        assert!(!config.contains("ANTHROPIC_API_KEY"));
        assert!(!config.contains("CLAUDE_CODE_OAUTH_TOKEN"));

        use metis_server::config::AppConfig;
        let app_config: AppConfig = serde_yaml_ng::from_str(&config)
            .expect("generated config should deserialize into AppConfig");

        assert_eq!(app_config.job.default_model.as_deref(), Some("gpt-4o"));
        assert_eq!(
            app_config.metis.openai_api_key.as_deref(),
            Some("sk-test-openai-key")
        );
        assert!(app_config.metis.anthropic_api_key.is_none());
        assert!(app_config.metis.claude_code_oauth_token.is_none());
    }

    #[test]
    fn render_server_config_with_claude_model_and_anthropic_key() {
        use base64::Engine;
        let encryption_key = base64::engine::general_purpose::STANDARD.encode([42u8; 32]);

        let keys = ApiKeys {
            anthropic_api_key: Some("sk-ant-test-key".to_string()),
            ..Default::default()
        };

        let config = render_server_config(
            &encryption_key,
            "ghp_test123",
            Path::new("/tmp/test.db"),
            Path::new("/tmp/auth-token"),
            "docker",
            Some("opus"),
            &keys,
            None,
            None,
        );

        assert!(config.contains("default_model: opus"));
        assert!(config.contains("ANTHROPIC_API_KEY: sk-ant-test-key"));
        assert!(!config.contains("OPENAI_API_KEY"));
        assert!(!config.contains("CLAUDE_CODE_OAUTH_TOKEN"));

        use metis_server::config::AppConfig;
        let app_config: AppConfig = serde_yaml_ng::from_str(&config)
            .expect("generated config should deserialize into AppConfig");

        assert_eq!(app_config.job.default_model.as_deref(), Some("opus"));
        assert_eq!(
            app_config.metis.anthropic_api_key.as_deref(),
            Some("sk-ant-test-key")
        );
        assert!(app_config.metis.openai_api_key.is_none());
        assert!(app_config.metis.claude_code_oauth_token.is_none());
    }

    #[test]
    fn render_server_config_with_claude_oauth_token() {
        use base64::Engine;
        let encryption_key = base64::engine::general_purpose::STANDARD.encode([42u8; 32]);

        let keys = ApiKeys {
            claude_code_oauth_token: Some("oauth-token-test".to_string()),
            ..Default::default()
        };

        let config = render_server_config(
            &encryption_key,
            "ghp_test123",
            Path::new("/tmp/test.db"),
            Path::new("/tmp/auth-token"),
            "docker",
            Some("opus"),
            &keys,
            None,
            None,
        );

        assert!(config.contains("default_model: opus"));
        assert!(config.contains("CLAUDE_CODE_OAUTH_TOKEN: oauth-token-test"));
        assert!(!config.contains("OPENAI_API_KEY"));
        assert!(!config.contains("ANTHROPIC_API_KEY"));

        use metis_server::config::AppConfig;
        let app_config: AppConfig = serde_yaml_ng::from_str(&config)
            .expect("generated config should deserialize into AppConfig");

        assert_eq!(app_config.job.default_model.as_deref(), Some("opus"));
        assert_eq!(
            app_config.metis.claude_code_oauth_token.as_deref(),
            Some("oauth-token-test")
        );
        assert!(app_config.metis.openai_api_key.is_none());
        assert!(app_config.metis.anthropic_api_key.is_none());
    }

    #[test]
    fn render_server_config_with_custom_username() {
        use base64::Engine;
        let encryption_key = base64::engine::general_purpose::STANDARD.encode([42u8; 32]);

        let config = render_server_config(
            &encryption_key,
            "ghp_test123",
            Path::new("/tmp/test.db"),
            Path::new("/tmp/auth-token"),
            "docker",
            None,
            &ApiKeys::default(),
            Some("alice"),
            None,
        );

        assert!(config.contains("username: alice"));

        use metis_server::config::AppConfig;
        let app_config: AppConfig = serde_yaml_ng::from_str(&config)
            .expect("generated config should deserialize into AppConfig");

        assert_eq!(app_config.auth.local_username(), Some("alice"));
    }

    #[test]
    fn store_auth_token_and_set_default_marks_local_server() {
        let dir = std::env::temp_dir().join(format!("metis-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let config_path = dir.join("config.toml");
        // Clean up any previous run.
        let _ = fs::remove_file(&config_path);

        // Simulate what cmd_init does: store auth token then set default server.
        config::store_auth_token(&config_path, LOCAL_SERVER_URL, "test-token")
            .expect("store auth token");
        let mut cli_config = config::AppConfig::load(&config_path).expect("load config");
        cli_config
            .set_default_server(LOCAL_SERVER_URL)
            .expect("set default server");
        cli_config.write_to(&config_path).expect("write config");

        // Reload and verify only one server entry exists.
        let reloaded = config::AppConfig::load(&config_path).expect("reload config");
        assert_eq!(
            reloaded.servers.len(),
            1,
            "should contain exactly one server entry"
        );
        let default = reloaded.default_server().expect("default server");
        assert_eq!(default.url, LOCAL_SERVER_URL);
        assert_eq!(default.auth_token.as_deref(), Some("test-token"));
        assert!(default.default);

        // Verify no staging server URL is present.
        let contents = fs::read_to_string(&config_path).expect("read config file");
        assert!(
            !contents.contains("metis-staging"),
            "config should not contain staging server URL"
        );

        // Clean up.
        let _ = fs::remove_dir_all(&dir);
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
