use std::path::Path;

use super::S3Error;

/// Computes an S3-style ETag (MD5 hash in quoted format) from bytes.
pub fn compute_etag(bytes: &[u8]) -> String {
    format!("\"{:x}\"", md5::compute(bytes))
}

/// Computes an S3-style ETag from a file path by streaming the file contents.
pub fn compute_etag_from_path(path: &Path) -> Result<String, std::io::Error> {
    use std::io::Read;

    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(file);
    let mut context = md5::Context::new();
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        context.consume(&buffer[..bytes_read]);
    }

    let digest = context.compute();
    Ok(format!("\"{digest:x}\""))
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

/// Reads an ETag from the metadata cache, falling back to computing it from the object file.
/// Used for backwards compatibility with objects created before ETag caching.
pub async fn read_etag_with_fallback(
    metadata_path: &Path,
    object_path: &Path,
) -> Result<String, std::io::Error> {
    // Try to read from cache first
    if let Some(etag) = read_cached_etag(metadata_path).await {
        return Ok(etag);
    }

    // Fall back to computing ETag from the object file
    compute_etag_from_path(object_path)
}

/// Synchronous version of read_etag_with_fallback for use in spawn_blocking contexts.
pub fn read_etag_with_fallback_sync(metadata_path: &Path, object_path: &Path) -> Option<String> {
    // Try to read from cache first
    if let Ok(contents) = std::fs::read_to_string(metadata_path) {
        return Some(contents);
    }

    // Fall back to computing ETag from the object file
    compute_etag_from_path(object_path).ok()
}
