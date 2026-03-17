use std::path::Path;

use super::S3Error;

/// Computes an S3-style ETag (MD5 hash in quoted format) from bytes.
pub fn compute_etag(bytes: &[u8]) -> String {
    format!("\"{:x}\"", md5::compute(bytes))
}

/// Writes an ETag to the metadata cache for an object.
pub async fn write_etag_metadata(metadata_path: &Path, etag: &str) -> Result<(), S3Error> {
    if let Some(parent) = metadata_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| S3Error::io("create metadata directory", e))?;
    }
    tokio::fs::write(metadata_path, etag)
        .await
        .map_err(|e| S3Error::io("write ETag metadata", e))?;
    Ok(())
}

/// Reads an ETag from the metadata cache.
/// Returns None if the cache file doesn't exist.
pub async fn read_cached_etag(metadata_path: &Path) -> Option<String> {
    tokio::fs::read_to_string(metadata_path).await.ok()
}
