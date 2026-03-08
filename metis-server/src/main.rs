use anyhow::Result;
use clap::{Parser, Subcommand};

mod init;

#[derive(Parser)]
#[command(name = "metis-server", version, about = "Metis server")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a local Metis environment: generate server config and
    /// configure the CLI.
    Init(init::InitArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Init(args)) => init::run(&args).await,
        None => metis_server::run().await,
    }
}
