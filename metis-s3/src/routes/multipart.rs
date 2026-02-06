use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::s3::S3State;
use crate::util::{
    MultipartQuery, MultipartUploadMetadata, S3_XML_NAMESPACE, S3Error, compute_etag,
    parse_complete_multipart_request, s3_error, sanitize_key, validate_bucket, write_etag_metadata,
    xml_escape,
};

/// POST /:bucket/*key?uploads - CreateMultipartUpload
pub async fn create_multipart_upload(
    State(state): State<Arc<S3State>>,
    Path((bucket, key)): Path<(String, String)>,
) -> Response {
    info!(bucket = %bucket, key = %key, "create_multipart_upload");

    // Validate bucket and key
    if let Err(err) = validate_bucket(&bucket) {
        error!(bucket = %bucket, error = %err.message, "create_multipart_upload failed: {}", err.code);
        return err.into_response();
    }
    if let Err(err) = sanitize_key(&key) {
        error!(key = %key, error = %err.message, "create_multipart_upload failed: {}", err.code);
        return err.into_response();
    }

    // Generate upload ID
    let upload_id = Uuid::new_v4().to_string();
    let upload_dir = state.upload_dir(&upload_id);

    // Create upload staging directory
    if let Err(err) = tokio::fs::create_dir_all(&upload_dir).await {
        error!(upload_id = %upload_id, error = %err, "create_multipart_upload failed: creating upload directory");
        return S3Error::io("creating upload directory", err).into_response();
    }

    // Store upload metadata
    let metadata = MultipartUploadMetadata {
        bucket: bucket.clone(),
        key: key.clone(),
        upload_id: upload_id.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    let metadata_path = state.upload_metadata_path(&upload_id);
    let metadata_json = match serde_json::to_string(&metadata) {
        Ok(json) => json,
        Err(err) => {
            error!(upload_id = %upload_id, error = %err, "create_multipart_upload failed: serializing metadata");
            return S3Error::io("serializing metadata", err).into_response();
        }
    };

    if let Err(err) = tokio::fs::write(&metadata_path, metadata_json).await {
        error!(upload_id = %upload_id, error = %err, "create_multipart_upload failed: writing metadata");
        return S3Error::io("writing metadata", err).into_response();
    }

    // Return XML response per AWS S3 API spec
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<InitiateMultipartUploadResult xmlns=\"{S3_XML_NAMESPACE}\">\n  <Bucket>{}</Bucket>\n  <Key>{}</Key>\n  <UploadId>{}</UploadId>\n</InitiateMultipartUploadResult>\n",
        xml_escape(&bucket),
        xml_escape(&key),
        xml_escape(&upload_id)
    );

    let mut response = Response::new(axum::body::Body::from(xml));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/xml"),
    );
    response
}

/// PUT /:bucket/*key?partNumber=N&uploadId=X - UploadPart
pub async fn upload_part(
    State(state): State<Arc<S3State>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(query): Query<MultipartQuery>,
    body: Bytes,
) -> Response {
    let upload_id = match &query.upload_id {
        Some(id) => id,
        None => {
            error!(bucket = %bucket, key = %key, "upload_part failed: missing uploadId");
            return s3_error(
                StatusCode::BAD_REQUEST,
                "InvalidRequest",
                "uploadId is required",
            );
        }
    };

    let part_number = match query.part_number {
        Some(n) if (1..=10000).contains(&n) => n,
        Some(n) => {
            error!(bucket = %bucket, key = %key, part_number = n, "upload_part failed: invalid part number");
            return s3_error(
                StatusCode::BAD_REQUEST,
                "InvalidPartNumber",
                "Part number must be between 1 and 10000",
            );
        }
        None => {
            error!(bucket = %bucket, key = %key, "upload_part failed: missing partNumber");
            return s3_error(
                StatusCode::BAD_REQUEST,
                "InvalidRequest",
                "partNumber is required",
            );
        }
    };

    info!(
        bucket = %bucket,
        key = %key,
        upload_id = %upload_id,
        part_number = part_number,
        body_size = body.len(),
        "upload_part"
    );

    // Verify upload exists
    let metadata_path = state.upload_metadata_path(upload_id);
    if !metadata_path.exists() {
        error!(upload_id = %upload_id, "upload_part failed: upload not found");
        return s3_error(StatusCode::NOT_FOUND, "NoSuchUpload", "Upload not found");
    }

    // Verify bucket/key match
    let metadata_bytes = match tokio::fs::read(&metadata_path).await {
        Ok(bytes) => bytes,
        Err(err) => {
            error!(upload_id = %upload_id, error = %err, "upload_part failed: reading metadata");
            return S3Error::io("reading metadata", err).into_response();
        }
    };

    let metadata: MultipartUploadMetadata = match serde_json::from_slice(&metadata_bytes) {
        Ok(m) => m,
        Err(err) => {
            error!(upload_id = %upload_id, error = %err, "upload_part failed: parsing metadata");
            return S3Error::io("parsing metadata", err).into_response();
        }
    };

    if metadata.bucket != bucket || metadata.key != key {
        error!(
            upload_id = %upload_id,
            expected_bucket = %metadata.bucket,
            expected_key = %metadata.key,
            actual_bucket = %bucket,
            actual_key = %key,
            "upload_part failed: bucket/key mismatch"
        );
        return s3_error(
            StatusCode::NOT_FOUND,
            "NoSuchUpload",
            "Upload not found for this bucket/key",
        );
    }

    // Write part to staging
    let part_path = state.part_path(upload_id, part_number);
    if let Err(err) = tokio::fs::write(&part_path, &body).await {
        error!(upload_id = %upload_id, part_number = part_number, error = %err, "upload_part failed: writing part");
        return S3Error::io("writing part", err).into_response();
    }

    // Compute ETag for this part
    let etag = compute_etag(&body);

    // Store ETag in a separate file for later use
    let etag_path = state
        .upload_dir(upload_id)
        .join(format!("etag-{part_number:05}"));
    if let Err(err) = tokio::fs::write(&etag_path, &etag).await {
        error!(upload_id = %upload_id, part_number = part_number, error = %err, "upload_part failed: writing etag");
        return S3Error::io("writing etag", err).into_response();
    }

    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        axum::http::header::ETAG,
        HeaderValue::from_str(&etag).unwrap_or_else(|_| HeaderValue::from_static("\"\"")),
    );
    response
}

/// POST /:bucket/*key?uploadId=X - CompleteMultipartUpload
pub async fn complete_multipart_upload(
    State(state): State<Arc<S3State>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(query): Query<MultipartQuery>,
    body: Bytes,
) -> Response {
    let upload_id = match &query.upload_id {
        Some(id) => id,
        None => {
            error!(bucket = %bucket, key = %key, "complete_multipart_upload failed: missing uploadId");
            return s3_error(
                StatusCode::BAD_REQUEST,
                "InvalidRequest",
                "uploadId is required",
            );
        }
    };

    info!(bucket = %bucket, key = %key, upload_id = %upload_id, "complete_multipart_upload");

    // Verify upload exists
    let upload_metadata_path = state.upload_metadata_path(upload_id);
    if !upload_metadata_path.exists() {
        error!(upload_id = %upload_id, "complete_multipart_upload failed: upload not found");
        return s3_error(StatusCode::NOT_FOUND, "NoSuchUpload", "Upload not found");
    }

    // Verify bucket/key match
    let metadata_bytes = match tokio::fs::read(&upload_metadata_path).await {
        Ok(bytes) => bytes,
        Err(err) => {
            error!(upload_id = %upload_id, error = %err, "complete_multipart_upload failed: reading metadata");
            return S3Error::io("reading metadata", err).into_response();
        }
    };

    let upload_metadata: MultipartUploadMetadata = match serde_json::from_slice(&metadata_bytes) {
        Ok(m) => m,
        Err(err) => {
            error!(upload_id = %upload_id, error = %err, "complete_multipart_upload failed: parsing metadata");
            return S3Error::io("parsing metadata", err).into_response();
        }
    };

    if upload_metadata.bucket != bucket || upload_metadata.key != key {
        error!(upload_id = %upload_id, "complete_multipart_upload failed: bucket/key mismatch");
        return s3_error(
            StatusCode::NOT_FOUND,
            "NoSuchUpload",
            "Upload not found for this bucket/key",
        );
    }

    // Parse request body to get part list
    let parts = match parse_complete_multipart_request(&body) {
        Ok(parts) => parts,
        Err(err) => {
            error!(upload_id = %upload_id, error = %err.message, "complete_multipart_upload failed: {}", err.code);
            return err.into_response();
        }
    };

    if parts.is_empty() {
        error!(upload_id = %upload_id, "complete_multipart_upload failed: no parts specified");
        return s3_error(
            StatusCode::BAD_REQUEST,
            "MalformedXML",
            "At least one part must be specified",
        );
    }

    // Verify parts are in order and exist with matching ETags
    let mut last_part_number = 0u32;
    for part in &parts {
        if part.part_number <= last_part_number {
            error!(upload_id = %upload_id, part_number = part.part_number, "complete_multipart_upload failed: parts not in order");
            return s3_error(
                StatusCode::BAD_REQUEST,
                "InvalidPartOrder",
                "Part numbers must be in ascending order",
            );
        }
        last_part_number = part.part_number;

        let part_path = state.part_path(upload_id, part.part_number);
        if !part_path.exists() {
            error!(upload_id = %upload_id, part_number = part.part_number, "complete_multipart_upload failed: part not found");
            return s3_error(
                StatusCode::NOT_FOUND,
                "InvalidPart",
                &format!("Part {} not found", part.part_number),
            );
        }

        // Verify ETag matches (optional but recommended for data integrity)
        let etag_path = state
            .upload_dir(upload_id)
            .join(format!("etag-{:05}", part.part_number));
        if let Ok(stored_etag) = tokio::fs::read_to_string(&etag_path).await {
            if stored_etag != part.etag {
                error!(
                    upload_id = %upload_id,
                    part_number = part.part_number,
                    expected = %stored_etag,
                    actual = %part.etag,
                    "complete_multipart_upload failed: ETag mismatch"
                );
                return s3_error(
                    StatusCode::BAD_REQUEST,
                    "InvalidPart",
                    &format!(
                        "Part {} ETag mismatch: expected {}, got {}",
                        part.part_number, stored_etag, part.etag
                    ),
                );
            }
        }
    }

    // Create target object path
    let object_path = match state.object_path(&bucket, &key) {
        Ok(path) => path,
        Err(err) => {
            error!(bucket = %bucket, key = %key, error = %err.message, "complete_multipart_upload failed: {}", err.code);
            return err.into_response();
        }
    };

    if let Some(parent) = object_path.parent() {
        if let Err(err) = tokio::fs::create_dir_all(parent).await {
            error!(error = %err, "complete_multipart_upload failed: creating object directory");
            return S3Error::io("creating object directory", err).into_response();
        }
    }

    // Concatenate parts into final object
    let mut final_file = match tokio::fs::File::create(&object_path).await {
        Ok(f) => f,
        Err(err) => {
            error!(error = %err, "complete_multipart_upload failed: creating final object");
            return S3Error::io("creating final object", err).into_response();
        }
    };

    let mut etag_hashes = Vec::new();
    for part in &parts {
        let part_path = state.part_path(upload_id, part.part_number);
        let part_data = match tokio::fs::read(&part_path).await {
            Ok(data) => data,
            Err(err) => {
                error!(part_number = part.part_number, error = %err, "complete_multipart_upload failed: reading part");
                return S3Error::io("reading part", err).into_response();
            }
        };

        // Collect MD5 hash bytes (without quotes) for final ETag calculation
        let part_md5 = md5::compute(&part_data);
        etag_hashes.extend_from_slice(&part_md5.0);

        if let Err(err) = final_file.write_all(&part_data).await {
            error!(part_number = part.part_number, error = %err, "complete_multipart_upload failed: writing part to final object");
            return S3Error::io("writing part to final object", err).into_response();
        }
    }

    if let Err(err) = final_file.flush().await {
        error!(error = %err, "complete_multipart_upload failed: flushing final object");
        return S3Error::io("flushing final object", err).into_response();
    }
    drop(final_file);

    // Calculate final ETag: MD5 of concatenated part MD5s + "-" + part count
    let final_md5 = md5::compute(&etag_hashes);
    let final_etag = format!("\"{:x}-{}\"", final_md5, parts.len());

    // Write ETag to metadata cache
    let metadata_path = match state.metadata_path(&bucket, &key) {
        Ok(p) => p,
        Err(err) => {
            error!(
                bucket = %bucket,
                key = %key,
                error = %err.message,
                "complete_multipart_upload: failed to get metadata path"
            );
            return err.into_response();
        }
    };
    if let Err(err) = write_etag_metadata(&metadata_path, &final_etag).await {
        error!(
            bucket = %bucket,
            key = %key,
            error = %err.message,
            "complete_multipart_upload: failed to write ETag to metadata cache"
        );
        return err.into_response();
    }

    // Clean up staging directory
    let upload_dir = state.upload_dir(upload_id);
    if let Err(err) = tokio::fs::remove_dir_all(&upload_dir).await {
        warn!(upload_id = %upload_id, error = %err, "complete_multipart_upload: failed to cleanup staging directory");
    }

    // Return XML response
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<CompleteMultipartUploadResult xmlns=\"{S3_XML_NAMESPACE}\">\n  <Location>/{}/{}</Location>\n  <Bucket>{}</Bucket>\n  <Key>{}</Key>\n  <ETag>{}</ETag>\n</CompleteMultipartUploadResult>\n",
        xml_escape(&bucket),
        xml_escape(&key),
        xml_escape(&bucket),
        xml_escape(&key),
        xml_escape(&final_etag)
    );

    let mut response = Response::new(axum::body::Body::from(xml));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/xml"),
    );
    response.headers_mut().insert(
        axum::http::header::ETAG,
        HeaderValue::from_str(&final_etag).unwrap_or_else(|_| HeaderValue::from_static("\"\"")),
    );
    response
}

/// DELETE /:bucket/*key?uploadId=X - AbortMultipartUpload
pub async fn abort_multipart_upload(
    State(state): State<Arc<S3State>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(query): Query<MultipartQuery>,
) -> Response {
    let upload_id = match &query.upload_id {
        Some(id) => id,
        None => {
            error!(bucket = %bucket, key = %key, "abort_multipart_upload failed: missing uploadId");
            return s3_error(
                StatusCode::BAD_REQUEST,
                "InvalidRequest",
                "uploadId is required",
            );
        }
    };

    info!(bucket = %bucket, key = %key, upload_id = %upload_id, "abort_multipart_upload");

    // Verify upload exists
    let metadata_path = state.upload_metadata_path(upload_id);
    if !metadata_path.exists() {
        error!(upload_id = %upload_id, "abort_multipart_upload failed: upload not found");
        return s3_error(StatusCode::NOT_FOUND, "NoSuchUpload", "Upload not found");
    }

    // Verify bucket/key match
    let metadata_bytes = match tokio::fs::read(&metadata_path).await {
        Ok(bytes) => bytes,
        Err(err) => {
            error!(upload_id = %upload_id, error = %err, "abort_multipart_upload failed: reading metadata");
            return S3Error::io("reading metadata", err).into_response();
        }
    };

    let metadata: MultipartUploadMetadata = match serde_json::from_slice(&metadata_bytes) {
        Ok(m) => m,
        Err(err) => {
            error!(upload_id = %upload_id, error = %err, "abort_multipart_upload failed: parsing metadata");
            return S3Error::io("parsing metadata", err).into_response();
        }
    };

    if metadata.bucket != bucket || metadata.key != key {
        error!(upload_id = %upload_id, "abort_multipart_upload failed: bucket/key mismatch");
        return s3_error(
            StatusCode::NOT_FOUND,
            "NoSuchUpload",
            "Upload not found for this bucket/key",
        );
    }

    // Remove staging directory
    let upload_dir = state.upload_dir(upload_id);
    if let Err(err) = tokio::fs::remove_dir_all(&upload_dir).await {
        error!(upload_id = %upload_id, error = %err, "abort_multipart_upload failed: removing staging directory");
        return S3Error::io("removing staging directory", err).into_response();
    }

    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;
    response
}
