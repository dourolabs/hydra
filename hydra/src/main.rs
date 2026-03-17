use anyhow::{Context, Result};
use clap::Parser;
use hydra::cli::Cli;

fn main() -> Result<()> {
    let cli = Cli::parse();

    let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
    rt.block_on(hydra::cli::run(cli))
}
