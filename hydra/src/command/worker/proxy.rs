use crate::client::HydraClientInterface;
use crate::command::output::CommandContext;
use crate::output_writer::write_stdout;
use anyhow::{Context, Result};
use clap::Subcommand;
use hydra_common::{
    constants::ENV_HYDRA_ID,
    sessions::{ProxyTarget, UpsertProxyTargetRequest},
    SessionId,
};

#[derive(Subcommand)]
pub enum ProxyCommand {
    /// Advertise that a server is listening on `--port`. Idempotent; calling
    /// `start` again with the same port replaces `--ready-path`.
    Start {
        /// Session id. Defaults to the worker container's `HYDRA_ID`.
        #[arg(long = "session-id", value_name = "ID", env = ENV_HYDRA_ID)]
        session_id: SessionId,

        /// TCP port the server is listening on inside the worker container.
        #[arg(long = "port", value_name = "PORT")]
        port: u16,

        /// HTTP path the proxy should probe to confirm the server is ready
        /// before forwarding user traffic. Omit if no readiness probe is
        /// needed.
        #[arg(long = "ready-path", value_name = "PATH")]
        ready_path: Option<String>,
    },
    /// Remove a previously advertised proxy target by port. Idempotent.
    Stop {
        /// Session id. Defaults to the worker container's `HYDRA_ID`.
        #[arg(long = "session-id", value_name = "ID", env = ENV_HYDRA_ID)]
        session_id: SessionId,

        /// TCP port that was previously passed to `start`.
        #[arg(long = "port", value_name = "PORT")]
        port: u16,
    },
    /// List the proxy targets currently advertised on this session.
    List {
        /// Session id. Defaults to the worker container's `HYDRA_ID`.
        #[arg(long = "session-id", value_name = "ID", env = ENV_HYDRA_ID)]
        session_id: SessionId,
    },
}

pub async fn run(
    client: &dyn HydraClientInterface,
    command: ProxyCommand,
    _context: &CommandContext,
) -> Result<()> {
    match command {
        ProxyCommand::Start {
            session_id,
            port,
            ready_path,
        } => {
            client
                .upsert_proxy_target(&session_id, &UpsertProxyTargetRequest { port, ready_path })
                .await
                .with_context(|| format!("failed to advertise proxy target on port {port}"))?;
            print_line(format!(
                "Advertised proxy target on port {port} for session '{session_id}'."
            ))?;
            Ok(())
        }
        ProxyCommand::Stop { session_id, port } => {
            client
                .delete_proxy_target(&session_id, port)
                .await
                .with_context(|| format!("failed to remove proxy target on port {port}"))?;
            print_line(format!(
                "Removed proxy target on port {port} for session '{session_id}'."
            ))?;
            Ok(())
        }
        ProxyCommand::List { session_id } => {
            let response = client
                .list_proxy_targets(&session_id)
                .await
                .with_context(|| format!("failed to list proxy targets for '{session_id}'"))?;
            print_targets(&response.targets)?;
            Ok(())
        }
    }
}

fn print_targets(targets: &[ProxyTarget]) -> Result<()> {
    if targets.is_empty() {
        return print_line("No proxy targets advertised.".to_string());
    }
    let mut buffer = Vec::new();
    use std::io::Write;
    for t in targets {
        let ready = t.ready_path.as_deref().unwrap_or("-");
        writeln!(&mut buffer, "port={} ready_path={}", t.port, ready)?;
    }
    write_stdout(&buffer)?;
    Ok(())
}

fn print_line(line: String) -> Result<()> {
    let mut bytes = line.into_bytes();
    bytes.push(b'\n');
    write_stdout(&bytes)?;
    Ok(())
}
