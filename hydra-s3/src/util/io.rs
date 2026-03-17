use axum::body::Bytes;
use std::path::Path;
use tokio::io::AsyncWriteExt;

/// Writes bytes to a file atomically.
pub async fn write_file(path: &Path, body: &Bytes) -> Result<(), std::io::Error> {
    let mut file = tokio::fs::File::create(path).await?;
    file.write_all(body).await?;
    file.flush().await?;
    Ok(())
}
