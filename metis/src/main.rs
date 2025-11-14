use clap::{Parser, Subcommand};

/// Top-level CLI options for the metis tool.
#[derive(Parser)]
#[command(name = "metis", version, about = "Utility CLI for AI orchestrator prototypes")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// Available subcommands for the CLI.
#[derive(Subcommand)]
enum Commands {
    /// Spawn a new orchestration worker.
    Spawn {
        /// Optional label to attach to the spawned worker.
        #[arg(short, long, value_name = "LABEL", default_value = "worker")]
        label: String,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Spawn { label } => {
            println!("Spawn request acknowledged for '{label}'.");
            println!("(Hook up your orchestration logic here.)");
        }
    }
}
