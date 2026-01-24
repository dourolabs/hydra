use anyhow::Result;
use std::path::Path;

use crate::client::{MetisClient, MetisClientUnauthenticated};

pub async fn run(client: &MetisClientUnauthenticated, _token_path: &Path) -> Result<()> {
    let _client = MetisClient::new(client.base_url().as_str(), String::new())?;
    Ok(())
}
