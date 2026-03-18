use std::path::{Component, Path as StdPath};

use super::error::S3Error;

pub fn validate_bucket(bucket: &str) -> Result<(), S3Error> {
    if bucket.trim().is_empty() {
        return Err(S3Error::bad_request(
            "InvalidBucketName",
            "Bucket name is required",
        ));
    }
    if bucket.contains('/') || bucket.contains('\\') {
        return Err(S3Error::bad_request(
            "InvalidBucketName",
            "Bucket name must not contain path separators",
        ));
    }
    if bucket == "." || bucket == ".." {
        return Err(S3Error::bad_request(
            "InvalidBucketName",
            "Bucket name must be a simple segment",
        ));
    }
    Ok(())
}

pub fn sanitize_key(key: &str) -> Result<String, S3Error> {
    if key.starts_with('/') {
        return Err(S3Error::bad_request(
            "InvalidObjectName",
            "Object key must not start with a slash",
        ));
    }
    let trimmed = key;
    if trimmed.trim().is_empty() {
        return Err(S3Error::bad_request(
            "InvalidObjectName",
            "Object key is required",
        ));
    }

    let path = StdPath::new(trimmed);
    if path.is_absolute() {
        return Err(S3Error::bad_request(
            "InvalidObjectName",
            "Object key must be relative",
        ));
    }

    for component in path.components() {
        match component {
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return Err(S3Error::bad_request(
                    "InvalidObjectName",
                    "Object key contains invalid path segments",
                ));
            }
            Component::CurDir => {
                return Err(S3Error::bad_request(
                    "InvalidObjectName",
                    "Object key contains invalid path segments",
                ));
            }
            Component::Normal(_) => {}
        }
    }

    Ok(path.to_string_lossy().replace('\\', "/"))
}

pub fn sanitize_prefix(prefix: &str) -> Result<String, S3Error> {
    if prefix.starts_with('/') {
        return Err(S3Error::bad_request(
            "InvalidRequest",
            "Prefix must not start with a slash",
        ));
    }
    let trimmed = prefix;
    if trimmed.trim().is_empty() {
        return Ok(String::new());
    }

    let path = StdPath::new(trimmed);
    if path.is_absolute() {
        return Err(S3Error::bad_request(
            "InvalidRequest",
            "Prefix must be relative",
        ));
    }

    for component in path.components() {
        match component {
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return Err(S3Error::bad_request(
                    "InvalidRequest",
                    "Prefix contains invalid path segments",
                ));
            }
            Component::CurDir => {
                return Err(S3Error::bad_request(
                    "InvalidRequest",
                    "Prefix contains invalid path segments",
                ));
            }
            Component::Normal(_) => {}
        }
    }

    Ok(path.to_string_lossy().replace('\\', "/"))
}
