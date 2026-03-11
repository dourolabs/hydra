mod bff;
mod frontend;
mod server;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use metis::cli;

use crate::server::ServerCommand;

#[derive(Parser)]
#[command(
    name = "metis",
    version,
    about = "Metis single-player: CLI + local server management"
)]
struct SinglePlayerCli {
    #[command(flatten)]
    cli: cli::Cli,

    #[command(subcommand)]
    command: Option<SinglePlayerCommands>,
}

#[derive(Subcommand)]
enum SinglePlayerCommands {
    /// Manage the local metis server.
    Server {
        #[command(subcommand)]
        command: ServerCommand,
    },
}

fn main() -> Result<()> {
    let sp_cli = SinglePlayerCli::parse();

    // Server commands run synchronously (before the tokio runtime) because
    // `server start` uses fork() which must happen before any threads exist.
    if let Some(SinglePlayerCommands::Server { command }) = sp_cli.command {
        return server::run(command);
    }

    // All other commands delegate to the shared CLI library.
    let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
    rt.block_on(cli::run(sp_cli.cli))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parse_server_init_subcommand() {
        let sp_cli = SinglePlayerCli::try_parse_from(["metis", "server", "init"]).expect("parse");
        match sp_cli.command {
            Some(SinglePlayerCommands::Server {
                command: ServerCommand::Init,
            }) => {}
            _ => panic!("expected Server Init"),
        }
    }

    #[test]
    fn parse_issues_subcommand() {
        let sp_cli = SinglePlayerCli::try_parse_from(["metis", "issues", "list"]).expect("parse");
        assert!(sp_cli.command.is_none());
        match sp_cli.cli.command {
            Some(cli::Commands::Issues { .. }) => {}
            _ => panic!("expected Issues"),
        }
    }

    #[test]
    fn parse_without_subcommand_defaults_to_dashboard() {
        let sp_cli = SinglePlayerCli::try_parse_from(["metis"]).expect("parse");
        assert!(sp_cli.command.is_none());
        let command = cli::resolve_command(sp_cli.cli.command);
        match command {
            cli::Commands::Dashboard => {}
            _ => panic!("expected dashboard default"),
        }
    }

    #[test]
    fn parse_server_logs_with_options() {
        let sp_cli =
            SinglePlayerCli::try_parse_from(["metis", "server", "logs", "-n", "100", "--follow"])
                .expect("parse");
        match sp_cli.command {
            Some(SinglePlayerCommands::Server {
                command: ServerCommand::Logs { lines, follow },
            }) => {
                assert_eq!(lines, 100);
                assert!(follow);
            }
            _ => panic!("expected Server Logs"),
        }
    }
}
