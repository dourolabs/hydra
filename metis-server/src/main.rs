use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    metis_server::run().await
}
