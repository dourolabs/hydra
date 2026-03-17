use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    hydra_server::run().await
}
