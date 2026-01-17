use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    metis::cli::run().await
}
