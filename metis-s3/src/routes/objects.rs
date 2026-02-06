use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header::RANGE},
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;
use tracing::{debug, error, info, warn};

use crate::s3::S3State;
use crate::util::{
    ByteRange, S3Error, compute_etag, range_not_satisfiable_response, read_etag_with_fallback,
    response_with_body, s3_error, streaming_partial_response, streaming_response,
    write_etag_metadata, write_file,
};

pub async fn put_object(
    State(state): State<Arc<S3State>>,
    Path((bucket, key)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    info!(
        bucket = %bucket,
        key = %key,
        body_size = body.len(),
        "put_object"
    );
    if body.is_empty() {
        warn!(bucket = %bucket, key = %key, "received empty PUT body");
    }

    let path = match state.object_path(&bucket, &key) {
        Ok(path) => path,
        Err(err) => {
            error!(
                bucket = %bucket,
                key = %key,
                error = %err.message,
                "put_object failed: {}", err.code
            );
            return err.into_response();
        }
    };

    if let Some(parent) = path.parent() {
        if let Err(err) = tokio::fs::create_dir_all(parent).await {
            error!(
                bucket = %bucket,
                key = %key,
                error = %err,
                "put_object failed: creating object directory"
            );
            return S3Error::io("creating object directory", err).into_response();
        }
    }

    if let Err(err) = write_file(&path, &body).await {
        error!(
            bucket = %bucket,
            key = %key,
            error = %err,
            "put_object failed: writing object"
        );
        return S3Error::io("writing object", err).into_response();
    }

    let etag = compute_etag(&body);

    // Write ETag to metadata cache (non-fatal on failure)
    match state.metadata_path(&bucket, &key) {
        Ok(metadata_path) => {
            if let Err(err) = write_etag_metadata(&metadata_path, &etag).await {
                warn!(
                    bucket = %bucket,
                    key = %key,
                    error = %err.message,
                    "failed to write ETag to metadata cache"
                );
            }
        }
        Err(err) => {
            warn!(
                bucket = %bucket,
                key = %key,
                error = %err.message,
                "failed to get metadata path for ETag cache"
            );
        }
    }

    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        axum::http::header::ETAG,
        HeaderValue::from_str(&etag).unwrap_or_else(|_| HeaderValue::from_static("\"\"")),
    );
    response
}

pub async fn get_object(
    State(state): State<Arc<S3State>>,
    headers: HeaderMap,
    Path((bucket, key)): Path<(String, String)>,
) -> Response {
    info!(bucket = %bucket, key = %key, "get_object");
    let path = match state.object_path(&bucket, &key) {
        Ok(path) => path,
        Err(err) => {
            error!(
                bucket = %bucket,
                key = %key,
                error = %err.message,
                "get_object failed: {}", err.code
            );
            return err.into_response();
        }
    };

    // Get metadata first to check existence and get file size
    let metadata = match tokio::fs::metadata(&path).await {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            error!(
                bucket = %bucket,
                key = %key,
                "get_object failed: NoSuchKey - Object not found"
            );
            return s3_error(StatusCode::NOT_FOUND, "NoSuchKey", "Object not found");
        }
        Err(err) => {
            error!(
                bucket = %bucket,
                key = %key,
                error = %err,
                "get_object failed: reading metadata"
            );
            return S3Error::io("reading metadata", err).into_response();
        }
    };

    let total_len = metadata.len();
    let last_modified = metadata.modified().ok();

    // Read ETag from metadata cache (falls back to computing if cache missing)
    let metadata_path = match state.metadata_path(&bucket, &key) {
        Ok(p) => p,
        Err(err) => {
            error!(
                bucket = %bucket,
                key = %key,
                error = %err.message,
                "get_object failed: getting metadata path"
            );
            return err.into_response();
        }
    };
    let etag = match read_etag_with_fallback(&metadata_path, &path).await {
        Ok(etag) => etag,
        Err(err) => {
            error!(
                bucket = %bucket,
                key = %key,
                error = %err,
                "get_object failed: reading etag"
            );
            return S3Error::io("reading etag", err).into_response();
        }
    };

    // Check for Range header
    let range_header = headers
        .get(RANGE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    if let Some(range_str) = range_header {
        match ByteRange::resolve(&range_str, total_len) {
            Some(range) => {
                // Open file and seek to the start position
                let mut file = match tokio::fs::File::open(&path).await {
                    Ok(f) => f,
                    Err(err) => {
                        error!(
                            bucket = %bucket,
                            key = %key,
                            error = %err,
                            "get_object failed: opening file for range request"
                        );
                        return S3Error::io("opening file", err).into_response();
                    }
                };

                if let Err(err) = file.seek(std::io::SeekFrom::Start(range.start)).await {
                    error!(
                        bucket = %bucket,
                        key = %key,
                        error = %err,
                        "get_object failed: seeking to range start"
                    );
                    return S3Error::io("seeking file", err).into_response();
                }

                let content_len = range.end - range.start + 1;
                let limited_reader = file.take(content_len);
                let stream = ReaderStream::new(limited_reader);
                let body = axum::body::Body::from_stream(stream);
                streaming_partial_response(body, last_modified, total_len, etag, range)
            }
            None => {
                // Range not satisfiable
                range_not_satisfiable_response(total_len)
            }
        }
    } else {
        // No Range header - stream full content
        let file = match tokio::fs::File::open(&path).await {
            Ok(f) => f,
            Err(err) => {
                error!(
                    bucket = %bucket,
                    key = %key,
                    error = %err,
                    "get_object failed: opening file"
                );
                return S3Error::io("opening file", err).into_response();
            }
        };

        let stream = ReaderStream::new(file);
        let body = axum::body::Body::from_stream(stream);
        streaming_response(body, last_modified, total_len, etag)
    }
}

pub async fn head_object(
    State(state): State<Arc<S3State>>,
    Path((bucket, key)): Path<(String, String)>,
) -> Response {
    info!(bucket = %bucket, key = %key, "head_object");
    let path = match state.object_path(&bucket, &key) {
        Ok(path) => path,
        Err(err) => {
            error!(
                bucket = %bucket,
                key = %key,
                error = %err.message,
                "head_object failed: {}", err.code
            );
            return err.into_response();
        }
    };

    let metadata = match tokio::fs::metadata(&path).await {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            error!(
                bucket = %bucket,
                key = %key,
                "head_object failed: NoSuchKey - Object not found"
            );
            return s3_error(StatusCode::NOT_FOUND, "NoSuchKey", "Object not found");
        }
        Err(err) => {
            error!(
                bucket = %bucket,
                key = %key,
                error = %err,
                "head_object failed: reading metadata"
            );
            return S3Error::io("reading metadata", err).into_response();
        }
    };

    // Read ETag from metadata cache (falls back to computing if cache missing)
    let metadata_path = match state.metadata_path(&bucket, &key) {
        Ok(p) => p,
        Err(err) => {
            error!(
                bucket = %bucket,
                key = %key,
                error = %err.message,
                "head_object failed: getting metadata path"
            );
            return err.into_response();
        }
    };
    let etag = match read_etag_with_fallback(&metadata_path, &path).await {
        Ok(etag) => etag,
        Err(err) => {
            error!(
                bucket = %bucket,
                key = %key,
                error = %err,
                "head_object failed: reading etag"
            );
            return S3Error::io("reading etag", err).into_response();
        }
    };
    response_with_body(Vec::new(), metadata.modified().ok(), metadata.len(), etag)
}

pub async fn delete_object(
    State(state): State<Arc<S3State>>,
    Path((bucket, key)): Path<(String, String)>,
) -> Response {
    info!(bucket = %bucket, key = %key, "delete_object");
    let path = match state.object_path(&bucket, &key) {
        Ok(path) => path,
        Err(err) => {
            error!(
                bucket = %bucket,
                key = %key,
                error = %err.message,
                "delete_object failed: {}", err.code
            );
            return err.into_response();
        }
    };

    match tokio::fs::remove_file(&path).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            error!(
                bucket = %bucket,
                key = %key,
                error = %err,
                "delete_object failed: deleting object"
            );
            return S3Error::io("deleting object", err).into_response();
        }
    }

    // Delete the ETag metadata file if it exists
    match state.metadata_path(&bucket, &key) {
        Ok(metadata_path) => {
            match tokio::fs::remove_file(&metadata_path).await {
                Ok(()) => {
                    debug!(
                        bucket = %bucket,
                        key = %key,
                        "delete_object: deleted ETag metadata file"
                    );
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    debug!(
                        bucket = %bucket,
                        key = %key,
                        "delete_object: ETag metadata file not found (already deleted or never existed)"
                    );
                }
                Err(err) => {
                    warn!(
                        bucket = %bucket,
                        key = %key,
                        error = %err,
                        "delete_object: failed to delete ETag metadata file"
                    );
                    // Don't fail the DELETE request - just warn about the orphaned metadata
                }
            }
        }
        Err(err) => {
            warn!(
                bucket = %bucket,
                key = %key,
                error = %err.message,
                "delete_object: failed to compute metadata path"
            );
            // Don't fail the DELETE request
        }
    }

    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;
    response
}
