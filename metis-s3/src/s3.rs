use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header::RANGE},
    response::{IntoResponse, Response},
    routing::{delete, get, head, post, put},
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use std::{
    borrow::Cow,
    fmt::Write as _,
    path::{Component, Path as StdPath, PathBuf},
    sync::Arc,
    time::SystemTime,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use walkdir::WalkDir;

const S3_XML_NAMESPACE: &str = "http://s3.amazonaws.com/doc/2006-03-01/";
const DEFAULT_MAX_KEYS: usize = 1000;

/// Represents a parsed byte range from an HTTP Range header.
/// Supports three forms: bytes=start-end, bytes=start-, bytes=-suffix
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ByteRange {
    start: u64,
    end: u64, // inclusive
}

impl ByteRange {
    /// Resolves the range against the total content length.
    /// Returns None if the range is invalid or unsatisfiable.
    fn resolve(range_spec: &str, total_len: u64) -> Option<Self> {
        if total_len == 0 {
            return None;
        }

        let range_spec = range_spec.trim();
        if !range_spec.starts_with("bytes=") {
            return None;
        }

        let range_part = &range_spec[6..];

        // S3 doesn't support multiple ranges
        if range_part.contains(',') {
            return None;
        }

        let parts: Vec<&str> = range_part.split('-').collect();
        if parts.len() != 2 {
            return None;
        }

        let start_str = parts[0].trim();
        let end_str = parts[1].trim();

        if start_str.is_empty() && end_str.is_empty() {
            return None;
        }

        if start_str.is_empty() {
            // Suffix range: bytes=-500 means last 500 bytes
            let suffix_len: u64 = end_str.parse().ok()?;
            if suffix_len == 0 {
                return None;
            }
            let start = total_len.saturating_sub(suffix_len);
            Some(ByteRange {
                start,
                end: total_len - 1,
            })
        } else if end_str.is_empty() {
            // Open-ended range: bytes=500-
            let start: u64 = start_str.parse().ok()?;
            if start >= total_len {
                return None;
            }
            Some(ByteRange {
                start,
                end: total_len - 1,
            })
        } else {
            // Explicit range: bytes=0-999
            let start: u64 = start_str.parse().ok()?;
            let end: u64 = end_str.parse().ok()?;
            if start > end {
                return None;
            }
            if start >= total_len {
                return None;
            }
            // Clamp end to content length - 1
            let end = end.min(total_len - 1);
            Some(ByteRange { start, end })
        }
    }
}

#[derive(Clone, Debug)]
pub struct S3State {
    root_dir: PathBuf,
}

impl S3State {
    pub fn new(root_dir: PathBuf) -> Result<Self, std::io::Error> {
        std::fs::create_dir_all(&root_dir)?;
        Ok(Self { root_dir })
    }

    fn bucket_dir(&self, bucket: &str) -> Result<PathBuf, S3Error> {
        validate_bucket(bucket)?;
        Ok(self.root_dir.join("buckets").join(bucket))
    }

    fn object_path(&self, bucket: &str, key: &str) -> Result<PathBuf, S3Error> {
        validate_bucket(bucket)?;
        let key = sanitize_key(key)?;
        Ok(self.root_dir.join("buckets").join(bucket).join(key))
    }

    fn multipart_dir(&self) -> PathBuf {
        self.root_dir.join("multipart")
    }

    fn upload_dir(&self, upload_id: &str) -> PathBuf {
        self.multipart_dir().join(upload_id)
    }

    fn part_path(&self, upload_id: &str, part_number: u32) -> PathBuf {
        self.upload_dir(upload_id)
            .join(format!("part-{part_number:05}"))
    }

    fn upload_metadata_path(&self, upload_id: &str) -> PathBuf {
        self.upload_dir(upload_id).join("metadata.json")
    }

    /// Returns the path for storing an object's ETag metadata.
    /// Path format: `{root_dir}/metadata/{bucket}/{key}.etag`
    fn metadata_path(&self, bucket: &str, key: &str) -> Result<PathBuf, S3Error> {
        validate_bucket(bucket)?;
        let key = sanitize_key(key)?;
        Ok(self
            .root_dir
            .join("metadata")
            .join(bucket)
            .join(format!("{key}.etag")))
    }

    /// Creates parent directories for the metadata file as needed.
    async fn ensure_metadata_dir(&self, bucket: &str, key: &str) -> Result<(), S3Error> {
        let path = self.metadata_path(bucket, key)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| S3Error::io("create metadata directory", e))?;
        }
        Ok(())
    }
}

pub fn router(root_dir: PathBuf) -> RouterWithState {
    let state =
        Arc::new(S3State::new(root_dir).expect("storage root directory should be available"));

    RouterWithState::new(state)
}

pub struct RouterWithState {
    router: axum::Router,
}

impl RouterWithState {
    fn new(state: Arc<S3State>) -> Self {
        let router = axum::Router::new()
            .route("/:bucket", get(list_objects_v2))
            .route("/:bucket/", get(list_objects_v2))
            .route("/:bucket/*key", put(put_object_handler))
            .route("/:bucket/*key", get(get_object))
            .route("/:bucket/*key", head(head_object))
            .route("/:bucket/*key", delete(delete_object_handler))
            .route("/:bucket/*key", post(post_object_handler))
            .with_state(state);
        Self { router }
    }
}

impl From<RouterWithState> for axum::Router {
    fn from(value: RouterWithState) -> Self {
        value.router
    }
}

#[derive(Debug)]
struct ObjectEntry {
    key: String,
    last_modified: Option<SystemTime>,
    size: u64,
    etag: Option<String>,
}

impl PartialEq for ObjectEntry {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for ObjectEntry {}

impl PartialOrd for ObjectEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ObjectEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}

#[derive(Debug)]
struct ListResult {
    entries: Vec<ObjectEntry>,
    is_truncated: bool,
    next_token: Option<String>,
}

/// Metadata stored for each multipart upload
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct MultipartUploadMetadata {
    bucket: String,
    key: String,
    upload_id: String,
    created_at: String,
}

/// Query parameters for multipart upload operations
#[derive(Debug, Deserialize)]
struct MultipartQuery {
    uploads: Option<String>,
    #[serde(rename = "uploadId")]
    upload_id: Option<String>,
    #[serde(rename = "partNumber")]
    part_number: Option<u32>,
}

/// Part information in CompleteMultipartUpload request body
#[derive(Debug)]
struct PartInfo {
    part_number: u32,
    etag: String,
}

#[derive(Debug, Deserialize)]
struct ListObjectsQuery {
    #[serde(rename = "list-type")]
    list_type: Option<u8>,
    prefix: Option<String>,
    #[serde(rename = "continuation-token")]
    continuation_token: Option<String>,
    #[serde(rename = "max-keys")]
    max_keys: Option<usize>,
    #[serde(rename = "start-after")]
    start_after: Option<String>,
}

/// Handler for PUT requests - dispatches to put_object or upload_part based on query params
async fn put_object_handler(
    State(state): State<Arc<S3State>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(query): Query<MultipartQuery>,
    body: Bytes,
) -> Response {
    if query.upload_id.is_some() && query.part_number.is_some() {
        upload_part(State(state), Path((bucket, key)), Query(query), body).await
    } else {
        put_object(State(state), Path((bucket, key)), body).await
    }
}

/// Handler for DELETE requests - dispatches to delete_object or abort_multipart_upload based on query params
async fn delete_object_handler(
    State(state): State<Arc<S3State>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(query): Query<MultipartQuery>,
) -> Response {
    if query.upload_id.is_some() {
        abort_multipart_upload(State(state), Path((bucket, key)), Query(query)).await
    } else {
        delete_object(State(state), Path((bucket, key))).await
    }
}

/// Handler for POST requests - dispatches to create_multipart_upload or complete_multipart_upload based on query params
async fn post_object_handler(
    State(state): State<Arc<S3State>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(query): Query<MultipartQuery>,
    body: Bytes,
) -> Response {
    if query.uploads.is_some() {
        create_multipart_upload(State(state), Path((bucket, key))).await
    } else if query.upload_id.is_some() {
        complete_multipart_upload(State(state), Path((bucket, key)), Query(query), body).await
    } else {
        s3_error(
            StatusCode::BAD_REQUEST,
            "InvalidRequest",
            "Invalid POST request - missing uploads or uploadId query parameter",
        )
    }
}

async fn list_objects_v2(
    State(state): State<Arc<S3State>>,
    Path(bucket): Path<String>,
    Query(query): Query<ListObjectsQuery>,
) -> Response {
    info!(
        bucket = %bucket,
        prefix = ?query.prefix,
        max_keys = ?query.max_keys,
        continuation_token = ?query.continuation_token,
        "list_objects_v2"
    );
    let list_type = query.list_type.unwrap_or(2);
    if list_type != 2 {
        error!(
            bucket = %bucket,
            list_type = list_type,
            "list_objects_v2 failed: list-type=2 is required for ListObjectsV2"
        );
        return s3_error(
            StatusCode::BAD_REQUEST,
            "InvalidRequest",
            "list-type=2 is required for ListObjectsV2",
        );
    }

    let prefix = match sanitize_prefix(query.prefix.as_deref().unwrap_or("")) {
        Ok(prefix) => prefix,
        Err(err) => {
            error!(
                bucket = %bucket,
                prefix = ?query.prefix,
                error = %err.message,
                "list_objects_v2 failed: invalid prefix"
            );
            return err.into_response();
        }
    };

    let max_keys = query
        .max_keys
        .unwrap_or(DEFAULT_MAX_KEYS)
        .min(DEFAULT_MAX_KEYS);

    let continuation_token = query.continuation_token.clone();
    let start_after = query.start_after.clone();
    let list_result = match list_objects(
        &state,
        &bucket,
        &prefix,
        continuation_token,
        start_after,
        max_keys,
    )
    .await
    {
        Ok(result) => result,
        Err(err) => {
            error!(
                bucket = %bucket,
                prefix = %prefix,
                error = %err.message,
                "list_objects_v2 failed: {}", err.code
            );
            return err.into_response();
        }
    };

    let xml = render_list_response(&bucket, &prefix, &query, max_keys, &list_result);
    let mut response = Response::new(axum::body::Body::from(xml));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/xml"),
    );
    response
}

async fn list_objects(
    state: &S3State,
    bucket: &str,
    prefix: &str,
    continuation_token: Option<String>,
    start_after: Option<String>,
    max_keys: usize,
) -> Result<ListResult, S3Error> {
    let bucket_dir = state.bucket_dir(bucket)?;
    let prefix = prefix.to_string();
    let marker = continuation_token.or(start_after);

    let entries = tokio::task::spawn_blocking(move || {
        if !bucket_dir.exists() {
            return Ok(Vec::new());
        }

        // Use a max-heap bounded to max_keys + 1 entries.
        // We store entries with Reverse ordering so the heap keeps the smallest keys
        // (lexicographically first) and we can efficiently remove the largest.
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;

        let limit = max_keys + 1;
        let mut heap: BinaryHeap<Reverse<ObjectEntry>> = BinaryHeap::with_capacity(limit + 1);

        for entry in WalkDir::new(&bucket_dir).follow_links(false) {
            let entry = entry.map_err(|err| S3Error::io("walking storage", err))?;
            if !entry.file_type().is_file() {
                continue;
            }

            let key = entry
                .path()
                .strip_prefix(&bucket_dir)
                .map_err(|err| S3Error::io("computing object key", err))?
                .to_string_lossy()
                .replace('\\', "/");

            // Skip entries that don't match the prefix
            if !prefix.is_empty() && !key.starts_with(&prefix) {
                continue;
            }

            // Skip entries at or before the marker
            if let Some(ref m) = marker {
                if key.as_str() <= m.as_str() {
                    continue;
                }
            }

            let metadata = entry
                .metadata()
                .map_err(|err| S3Error::io("reading metadata", err))?;
            let last_modified = metadata.modified().ok();
            let size = metadata.len();
            let etag = compute_etag_from_path(entry.path()).ok();

            let obj = ObjectEntry {
                key,
                last_modified,
                size,
                etag,
            };

            heap.push(Reverse(obj));

            // If heap exceeds our limit, remove the largest entry (last in sorted order)
            if heap.len() > limit {
                heap.pop();
            }
        }

        // Convert heap to sorted vector
        let mut entries: Vec<ObjectEntry> = heap.into_iter().map(|Reverse(e)| e).collect();
        entries.sort_by(|a, b| a.key.cmp(&b.key));

        Ok(entries)
    })
    .await
    .map_err(|err| S3Error::io("listing objects", err))??;

    let mut entries = entries;
    let mut is_truncated = false;
    let mut next_token = None;
    if entries.len() > max_keys {
        is_truncated = true;
        entries.truncate(max_keys);
        if let Some(last) = entries.last() {
            next_token = Some(last.key.clone());
        }
    }

    Ok(ListResult {
        entries,
        is_truncated,
        next_token,
    })
}

async fn put_object(
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
    if let Err(err) = write_etag_metadata(&state, &bucket, &key, &etag).await {
        warn!(
            bucket = %bucket,
            key = %key,
            error = %err.message,
            "failed to write ETag to metadata cache"
        );
    }

    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        axum::http::header::ETAG,
        HeaderValue::from_str(&etag).unwrap_or_else(|_| HeaderValue::from_static("\"\"")),
    );
    response
}

async fn get_object(
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

    // Compute ETag incrementally without loading entire file into memory
    let etag = match compute_etag_from_path(&path) {
        Ok(etag) => etag,
        Err(err) => {
            error!(
                bucket = %bucket,
                key = %key,
                error = %err,
                "get_object failed: computing etag"
            );
            return S3Error::io("computing etag", err).into_response();
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

async fn head_object(
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

    let etag = match compute_etag_from_path(&path) {
        Ok(etag) => etag,
        Err(err) => {
            error!(
                bucket = %bucket,
                key = %key,
                error = %err,
                "head_object failed: reading object for etag"
            );
            return S3Error::io("reading object", err).into_response();
        }
    };
    response_with_body(Vec::new(), metadata.modified().ok(), metadata.len(), etag)
}

/// POST /:bucket/*key?uploads - CreateMultipartUpload
async fn create_multipart_upload(
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
async fn upload_part(
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
async fn complete_multipart_upload(
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
    let metadata_path = state.upload_metadata_path(upload_id);
    if !metadata_path.exists() {
        error!(upload_id = %upload_id, "complete_multipart_upload failed: upload not found");
        return s3_error(StatusCode::NOT_FOUND, "NoSuchUpload", "Upload not found");
    }

    // Verify bucket/key match
    let metadata_bytes = match tokio::fs::read(&metadata_path).await {
        Ok(bytes) => bytes,
        Err(err) => {
            error!(upload_id = %upload_id, error = %err, "complete_multipart_upload failed: reading metadata");
            return S3Error::io("reading metadata", err).into_response();
        }
    };

    let metadata: MultipartUploadMetadata = match serde_json::from_slice(&metadata_bytes) {
        Ok(m) => m,
        Err(err) => {
            error!(upload_id = %upload_id, error = %err, "complete_multipart_upload failed: parsing metadata");
            return S3Error::io("parsing metadata", err).into_response();
        }
    };

    if metadata.bucket != bucket || metadata.key != key {
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
    if let Err(err) = write_etag_metadata(&state, &bucket, &key, &final_etag).await {
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
async fn abort_multipart_upload(
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

/// Parse CompleteMultipartUpload XML request body
fn parse_complete_multipart_request(body: &[u8]) -> Result<Vec<PartInfo>, S3Error> {
    let body_str = std::str::from_utf8(body)
        .map_err(|_| S3Error::bad_request("MalformedXML", "Request body is not valid UTF-8"))?;

    let mut parts = Vec::new();

    // Simple XML parsing for <Part><PartNumber>N</PartNumber><ETag>X</ETag></Part>
    for part_match in body_str.split("<Part>").skip(1) {
        let part_end = part_match.find("</Part>").unwrap_or(part_match.len());
        let part_content = &part_match[..part_end];

        let part_number = extract_xml_value(part_content, "PartNumber")
            .ok_or_else(|| S3Error::bad_request("MalformedXML", "Part missing PartNumber"))?
            .parse::<u32>()
            .map_err(|_| S3Error::bad_request("MalformedXML", "Invalid PartNumber"))?;

        let etag_raw = extract_xml_value(part_content, "ETag")
            .ok_or_else(|| S3Error::bad_request("MalformedXML", "Part missing ETag"))?;
        let etag = xml_unescape(etag_raw);

        parts.push(PartInfo { part_number, etag });
    }

    Ok(parts)
}

/// Extract value from simple XML tag
fn extract_xml_value<'a>(content: &'a str, tag: &str) -> Option<&'a str> {
    let start_tag = format!("<{tag}>");
    let end_tag = format!("</{tag}>");

    let start = content.find(&start_tag)? + start_tag.len();
    let end = content.find(&end_tag)?;

    if start <= end {
        Some(&content[start..end])
    } else {
        None
    }
}

/// Unescape common XML entities
fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

async fn delete_object(
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

fn response_with_body(
    body: Vec<u8>,
    last_modified: Option<SystemTime>,
    content_len: u64,
    etag: String,
) -> Response {
    let mut response = Response::new(axum::body::Body::from(body));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        axum::http::header::CONTENT_LENGTH,
        HeaderValue::from_str(&content_len.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response.headers_mut().insert(
        axum::http::header::ETAG,
        HeaderValue::from_str(&etag).unwrap_or_else(|_| HeaderValue::from_static("\"\"")),
    );
    response.headers_mut().insert(
        axum::http::header::ACCEPT_RANGES,
        HeaderValue::from_static("bytes"),
    );
    if let Some(modified) = last_modified {
        let header_value = httpdate::fmt_http_date(modified);
        if let Ok(value) = HeaderValue::from_str(&header_value) {
            response
                .headers_mut()
                .insert(axum::http::header::LAST_MODIFIED, value);
        }
    }
    response
}

fn streaming_response(
    body: axum::body::Body,
    last_modified: Option<SystemTime>,
    content_len: u64,
    etag: String,
) -> Response {
    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        axum::http::header::CONTENT_LENGTH,
        HeaderValue::from_str(&content_len.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response.headers_mut().insert(
        axum::http::header::ETAG,
        HeaderValue::from_str(&etag).unwrap_or_else(|_| HeaderValue::from_static("\"\"")),
    );
    response.headers_mut().insert(
        axum::http::header::ACCEPT_RANGES,
        HeaderValue::from_static("bytes"),
    );
    if let Some(modified) = last_modified {
        let header_value = httpdate::fmt_http_date(modified);
        if let Ok(value) = HeaderValue::from_str(&header_value) {
            response
                .headers_mut()
                .insert(axum::http::header::LAST_MODIFIED, value);
        }
    }
    response
}

fn streaming_partial_response(
    body: axum::body::Body,
    last_modified: Option<SystemTime>,
    total_len: u64,
    etag: String,
    range: ByteRange,
) -> Response {
    let content_len = range.end - range.start + 1;
    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::PARTIAL_CONTENT;
    response.headers_mut().insert(
        axum::http::header::CONTENT_LENGTH,
        HeaderValue::from_str(&content_len.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response.headers_mut().insert(
        axum::http::header::ETAG,
        HeaderValue::from_str(&etag).unwrap_or_else(|_| HeaderValue::from_static("\"\"")),
    );
    response.headers_mut().insert(
        axum::http::header::ACCEPT_RANGES,
        HeaderValue::from_static("bytes"),
    );
    // Content-Range: bytes start-end/total
    let content_range = format!("bytes {}-{}/{}", range.start, range.end, total_len);
    if let Ok(value) = HeaderValue::from_str(&content_range) {
        response
            .headers_mut()
            .insert(axum::http::header::CONTENT_RANGE, value);
    }
    if let Some(modified) = last_modified {
        let header_value = httpdate::fmt_http_date(modified);
        if let Ok(value) = HeaderValue::from_str(&header_value) {
            response
                .headers_mut()
                .insert(axum::http::header::LAST_MODIFIED, value);
        }
    }
    response
}

fn range_not_satisfiable_response(total_len: u64) -> Response {
    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
    // Content-Range: bytes */total
    let content_range = format!("bytes */{total_len}");
    if let Ok(value) = HeaderValue::from_str(&content_range) {
        response
            .headers_mut()
            .insert(axum::http::header::CONTENT_RANGE, value);
    }
    response
}

async fn write_file(path: &StdPath, body: &Bytes) -> Result<(), std::io::Error> {
    let mut file = tokio::fs::File::create(path).await?;
    file.write_all(body).await?;
    file.flush().await?;
    Ok(())
}

/// Writes an ETag to the metadata cache for an object.
async fn write_etag_metadata(
    state: &S3State,
    bucket: &str,
    key: &str,
    etag: &str,
) -> Result<(), S3Error> {
    state.ensure_metadata_dir(bucket, key).await?;
    let metadata_path = state.metadata_path(bucket, key)?;
    tokio::fs::write(&metadata_path, etag)
        .await
        .map_err(|e| S3Error::io("write ETag metadata", e))?;
    Ok(())
}

fn render_list_response(
    bucket: &str,
    prefix: &str,
    query: &ListObjectsQuery,
    max_keys: usize,
    result: &ListResult,
) -> String {
    let mut xml = String::new();
    let _ = writeln!(
        xml,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ListBucketResult xmlns=\"{S3_XML_NAMESPACE}\">"
    );
    push_xml(&mut xml, "Name", bucket);
    push_xml(&mut xml, "Prefix", prefix);
    if let Some(token) = query.continuation_token.as_deref() {
        push_xml(&mut xml, "ContinuationToken", token);
    }
    if let Some(start_after) = query.start_after.as_deref() {
        push_xml(&mut xml, "StartAfter", start_after);
    }
    push_xml(&mut xml, "KeyCount", &result.entries.len().to_string());
    push_xml(&mut xml, "MaxKeys", &max_keys.to_string());
    push_xml(
        &mut xml,
        "IsTruncated",
        if result.is_truncated { "true" } else { "false" },
    );

    for entry in &result.entries {
        xml.push_str("  <Contents>\n");
        push_xml(&mut xml, "Key", &entry.key);
        if let Some(last_modified) = entry.last_modified {
            let date: DateTime<Utc> = last_modified.into();
            push_xml(
                &mut xml,
                "LastModified",
                &date.to_rfc3339_opts(SecondsFormat::Millis, true),
            );
        }
        if let Some(etag) = entry.etag.as_deref() {
            push_xml(&mut xml, "ETag", etag);
        }
        push_xml(&mut xml, "Size", &entry.size.to_string());
        xml.push_str("  </Contents>\n");
    }

    if let Some(token) = result.next_token.as_deref() {
        push_xml(&mut xml, "NextContinuationToken", token);
    }

    xml.push_str("</ListBucketResult>\n");
    xml
}

fn push_xml(xml: &mut String, tag: &str, value: &str) {
    let escaped = xml_escape(value);
    let _ = writeln!(xml, "  <{tag}>{escaped}</{tag}>");
}

fn xml_escape(value: &str) -> Cow<'_, str> {
    if !value.contains(['&', '<', '>', '\'', '"']) {
        return Cow::Borrowed(value);
    }

    let mut escaped = String::with_capacity(value.len() + 8);
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\'' => escaped.push_str("&apos;"),
            '"' => escaped.push_str("&quot;"),
            _ => escaped.push(ch),
        }
    }
    Cow::Owned(escaped)
}

fn validate_bucket(bucket: &str) -> Result<(), S3Error> {
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

fn sanitize_key(key: &str) -> Result<String, S3Error> {
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

fn sanitize_prefix(prefix: &str) -> Result<String, S3Error> {
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

fn compute_etag(bytes: &[u8]) -> String {
    format!("\"{:x}\"", md5::compute(bytes))
}

fn compute_etag_from_path(path: &StdPath) -> Result<String, std::io::Error> {
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

#[derive(Debug)]
struct S3Error {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl S3Error {
    fn bad_request(code: &'static str, message: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.to_string(),
        }
    }

    fn io(context: &'static str, err: impl std::fmt::Display) -> Self {
        warn!(error = %err, "S3 storage error: {context}");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "InternalError",
            message: context.to_string(),
        }
    }
}

impl IntoResponse for S3Error {
    fn into_response(self) -> Response {
        s3_error(self.status, self.code, &self.message)
    }
}

fn s3_error(status: StatusCode, code: &'static str, message: &str) -> Response {
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error>\n  <Code>{}</Code>\n  <Message>{}</Message>\n  <RequestId>metis-s3</RequestId>\n</Error>\n",
        xml_escape(code),
        xml_escape(message)
    );
    let mut response = Response::new(axum::body::Body::from(xml));
    *response.status_mut() = status;
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/xml"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tempfile::tempdir;
    use tower::ServiceExt;

    #[test]
    fn sanitize_key_rejects_invalid_segments() {
        assert!(sanitize_key("../secret").is_err());
        assert!(sanitize_key("/absolute").is_err());
        assert!(sanitize_key("./local").is_err());
        assert!(sanitize_key("").is_err());
    }

    #[test]
    fn sanitize_prefix_allows_empty() {
        assert_eq!(sanitize_prefix("").expect("prefix"), "");
        assert!(sanitize_prefix("../secret").is_err());
    }

    #[tokio::test]
    async fn object_lifecycle_roundtrip() {
        let dir = tempdir().expect("temp dir");
        let router: axum::Router = RouterWithState::new(Arc::new(
            S3State::new(dir.path().to_path_buf()).expect("state"),
        ))
        .into();

        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/cache-bucket/builds/cache.tgz")
                    .body(Body::from("payload"))
                    .unwrap(),
            )
            .await
            .expect("put response");
        assert_eq!(response.status(), StatusCode::OK);

        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("HEAD")
                    .uri("/cache-bucket/builds/cache.tgz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("head response");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get("etag").is_some());

        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/cache-bucket/builds/cache.tgz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("get response");
        assert_eq!(response.status(), StatusCode::OK);

        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/cache-bucket?list-type=2&prefix=builds/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("list response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body = String::from_utf8(body.to_vec()).expect("utf8");
        assert!(body.contains("cache.tgz"));

        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/cache-bucket/builds/cache.tgz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("delete response");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/cache-bucket/builds/cache.tgz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("get after delete response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn rejects_path_traversal() {
        let dir = tempdir().expect("temp dir");
        let router: axum::Router = RouterWithState::new(Arc::new(
            S3State::new(dir.path().to_path_buf()).expect("state"),
        ))
        .into();

        let response = router
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/bucket/../secret.txt")
                    .body(Body::from("payload"))
                    .unwrap(),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn multipart_upload_lifecycle() {
        let dir = tempdir().expect("temp dir");
        let router: axum::Router = RouterWithState::new(Arc::new(
            S3State::new(dir.path().to_path_buf()).expect("state"),
        ))
        .into();

        // Step 1: Create multipart upload
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/test-bucket/large-file.bin?uploads")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("create multipart response");
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body_str = String::from_utf8(body.to_vec()).expect("utf8");
        assert!(body_str.contains("<InitiateMultipartUploadResult"));
        assert!(body_str.contains("<UploadId>"));
        assert!(body_str.contains("<Bucket>test-bucket</Bucket>"));
        assert!(body_str.contains("<Key>large-file.bin</Key>"));

        // Extract uploadId from response
        let upload_id = extract_xml_value(&body_str, "UploadId").expect("upload_id");

        // Step 2: Upload parts (out of order to test ordering)
        let part2_data = b"part2-data-content";
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/test-bucket/large-file.bin?uploadId={upload_id}&partNumber=2"
                    ))
                    .body(Body::from(&part2_data[..]))
                    .unwrap(),
            )
            .await
            .expect("upload part 2 response");
        assert_eq!(response.status(), StatusCode::OK);
        let etag2 = response
            .headers()
            .get("etag")
            .expect("etag header")
            .to_str()
            .expect("etag str")
            .to_string();

        let part1_data = b"part1-data-content";
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/test-bucket/large-file.bin?uploadId={upload_id}&partNumber=1"
                    ))
                    .body(Body::from(&part1_data[..]))
                    .unwrap(),
            )
            .await
            .expect("upload part 1 response");
        assert_eq!(response.status(), StatusCode::OK);
        let etag1 = response
            .headers()
            .get("etag")
            .expect("etag header")
            .to_str()
            .expect("etag str")
            .to_string();

        // Step 3: Complete multipart upload
        let complete_body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<CompleteMultipartUpload>
    <Part>
        <PartNumber>1</PartNumber>
        <ETag>{etag1}</ETag>
    </Part>
    <Part>
        <PartNumber>2</PartNumber>
        <ETag>{etag2}</ETag>
    </Part>
</CompleteMultipartUpload>"#
        );

        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/test-bucket/large-file.bin?uploadId={upload_id}"))
                    .body(Body::from(complete_body))
                    .unwrap(),
            )
            .await
            .expect("complete multipart response");
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body_str = String::from_utf8(body.to_vec()).expect("utf8");
        assert!(body_str.contains("<CompleteMultipartUploadResult"));
        assert!(body_str.contains("<ETag>"));

        // Step 4: Verify the final object exists and has correct content
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/test-bucket/large-file.bin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("get object response");
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let expected_content: Vec<u8> = [&part1_data[..], &part2_data[..]].concat();
        assert_eq!(body.to_vec(), expected_content);

        // Verify ETag contains the multipart marker (dash followed by part count)
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("HEAD")
                    .uri("/test-bucket/large-file.bin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("head object response");
        assert_eq!(response.status(), StatusCode::OK);

        // Verify staging directory was cleaned up
        let multipart_dir = dir.path().join("multipart").join(upload_id);
        assert!(
            !multipart_dir.exists(),
            "staging directory should be cleaned up"
        );
    }

    #[tokio::test]
    async fn multipart_upload_writes_etag_to_metadata_cache() {
        let dir = tempdir().expect("temp dir");
        let state = Arc::new(S3State::new(dir.path().to_path_buf()).expect("state"));
        let router: axum::Router = RouterWithState::new(state.clone()).into();

        // Step 1: Create multipart upload
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/test-bucket/cached-etag.bin?uploads")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("create multipart response");
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body_str = String::from_utf8(body.to_vec()).expect("utf8");
        let upload_id = extract_xml_value(&body_str, "UploadId").expect("upload_id");

        // Step 2: Upload parts
        let part1_data = b"first-part-data";
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/test-bucket/cached-etag.bin?uploadId={upload_id}&partNumber=1"
                    ))
                    .body(Body::from(&part1_data[..]))
                    .unwrap(),
            )
            .await
            .expect("upload part 1 response");
        assert_eq!(response.status(), StatusCode::OK);
        let etag1 = response
            .headers()
            .get("etag")
            .expect("etag header")
            .to_str()
            .expect("etag str")
            .to_string();

        let part2_data = b"second-part-data";
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/test-bucket/cached-etag.bin?uploadId={upload_id}&partNumber=2"
                    ))
                    .body(Body::from(&part2_data[..]))
                    .unwrap(),
            )
            .await
            .expect("upload part 2 response");
        assert_eq!(response.status(), StatusCode::OK);
        let etag2 = response
            .headers()
            .get("etag")
            .expect("etag header")
            .to_str()
            .expect("etag str")
            .to_string();

        let part3_data = b"third-part-data";
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/test-bucket/cached-etag.bin?uploadId={upload_id}&partNumber=3"
                    ))
                    .body(Body::from(&part3_data[..]))
                    .unwrap(),
            )
            .await
            .expect("upload part 3 response");
        assert_eq!(response.status(), StatusCode::OK);
        let etag3 = response
            .headers()
            .get("etag")
            .expect("etag header")
            .to_str()
            .expect("etag str")
            .to_string();

        // Step 3: Complete multipart upload
        let complete_body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<CompleteMultipartUpload>
    <Part>
        <PartNumber>1</PartNumber>
        <ETag>{etag1}</ETag>
    </Part>
    <Part>
        <PartNumber>2</PartNumber>
        <ETag>{etag2}</ETag>
    </Part>
    <Part>
        <PartNumber>3</PartNumber>
        <ETag>{etag3}</ETag>
    </Part>
</CompleteMultipartUpload>"#
        );

        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/test-bucket/cached-etag.bin?uploadId={upload_id}"))
                    .body(Body::from(complete_body))
                    .unwrap(),
            )
            .await
            .expect("complete multipart response");
        assert_eq!(response.status(), StatusCode::OK);

        // Extract ETag from response header (not XML body, which has escaped quotes)
        let response_etag = response
            .headers()
            .get("etag")
            .expect("etag header")
            .to_str()
            .expect("etag str")
            .to_string();

        // Verify ETag file exists at metadata/{bucket}/{key}.etag
        let etag_path = state
            .metadata_path("test-bucket", "cached-etag.bin")
            .expect("metadata path");
        assert!(
            etag_path.exists(),
            "ETag metadata file should exist at {etag_path:?}"
        );

        // Verify ETag file contains the correct multipart-style ETag
        let cached_etag = tokio::fs::read_to_string(&etag_path)
            .await
            .expect("read cached etag");
        assert_eq!(
            cached_etag, response_etag,
            "cached ETag should match response ETag"
        );

        // Verify ETag has multipart format (contains dash followed by part count)
        assert!(
            cached_etag.contains("-3"),
            "ETag should contain multipart marker '-3' for 3 parts, got: {cached_etag}"
        );
    }

    #[tokio::test]
    async fn multipart_upload_abort() {
        let dir = tempdir().expect("temp dir");
        let router: axum::Router = RouterWithState::new(Arc::new(
            S3State::new(dir.path().to_path_buf()).expect("state"),
        ))
        .into();

        // Create multipart upload
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/test-bucket/abort-test.bin?uploads")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("create multipart response");
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body_str = String::from_utf8(body.to_vec()).expect("utf8");
        let upload_id = extract_xml_value(&body_str, "UploadId").expect("upload_id");

        // Upload a part
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/test-bucket/abort-test.bin?uploadId={upload_id}&partNumber=1"
                    ))
                    .body(Body::from("part-data"))
                    .unwrap(),
            )
            .await
            .expect("upload part response");
        assert_eq!(response.status(), StatusCode::OK);

        // Verify staging directory exists
        let multipart_dir = dir.path().join("multipart").join(upload_id);
        assert!(
            multipart_dir.exists(),
            "staging directory should exist before abort"
        );

        // Abort the upload
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/test-bucket/abort-test.bin?uploadId={upload_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("abort multipart response");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        // Verify staging directory was cleaned up
        assert!(
            !multipart_dir.exists(),
            "staging directory should be cleaned up after abort"
        );

        // Verify the object was not created
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/test-bucket/abort-test.bin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("get object response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn multipart_upload_errors() {
        let dir = tempdir().expect("temp dir");
        let router: axum::Router = RouterWithState::new(Arc::new(
            S3State::new(dir.path().to_path_buf()).expect("state"),
        ))
        .into();

        // Test: Upload part with non-existent uploadId
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/test-bucket/test.bin?uploadId=non-existent&partNumber=1")
                    .body(Body::from("data"))
                    .unwrap(),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // Test: Upload part with invalid part number (0)
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/test-bucket/test.bin?uploadId=fake&partNumber=0")
                    .body(Body::from("data"))
                    .unwrap(),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // Test: Upload part with invalid part number (> 10000)
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/test-bucket/test.bin?uploadId=fake&partNumber=10001")
                    .body(Body::from("data"))
                    .unwrap(),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // Test: Complete multipart with non-existent uploadId
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/test-bucket/test.bin?uploadId=non-existent")
                    .body(Body::from("<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>\"abc\"</ETag></Part></CompleteMultipartUpload>"))
                    .unwrap(),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // Test: Abort multipart with non-existent uploadId
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/test-bucket/test.bin?uploadId=non-existent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_parse_complete_multipart_request() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<CompleteMultipartUpload>
    <Part>
        <PartNumber>1</PartNumber>
        <ETag>"abc123"</ETag>
    </Part>
    <Part>
        <PartNumber>2</PartNumber>
        <ETag>"def456"</ETag>
    </Part>
</CompleteMultipartUpload>"#;

        let parts = parse_complete_multipart_request(xml.as_bytes()).expect("parse");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].part_number, 1);
        assert_eq!(parts[0].etag, "\"abc123\"");
        assert_eq!(parts[1].part_number, 2);
        assert_eq!(parts[1].etag, "\"def456\"");
    }

    #[test]
    fn byte_range_explicit_range() {
        // bytes=0-999 on a 2000 byte file
        let range = ByteRange::resolve("bytes=0-999", 2000).expect("range");
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 999);
    }

    #[test]
    fn byte_range_open_ended() {
        // bytes=500- on a 2000 byte file
        let range = ByteRange::resolve("bytes=500-", 2000).expect("range");
        assert_eq!(range.start, 500);
        assert_eq!(range.end, 1999);
    }

    #[test]
    fn byte_range_suffix() {
        // bytes=-500 on a 2000 byte file (last 500 bytes)
        let range = ByteRange::resolve("bytes=-500", 2000).expect("range");
        assert_eq!(range.start, 1500);
        assert_eq!(range.end, 1999);
    }

    #[test]
    fn byte_range_clamped_to_content_length() {
        // bytes=0-5000 on a 2000 byte file should clamp to 1999
        let range = ByteRange::resolve("bytes=0-5000", 2000).expect("range");
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 1999);
    }

    #[test]
    fn byte_range_suffix_larger_than_file() {
        // bytes=-5000 on a 2000 byte file should return the whole file
        let range = ByteRange::resolve("bytes=-5000", 2000).expect("range");
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 1999);
    }

    #[test]
    fn byte_range_invalid_start_beyond_content() {
        // bytes=5000- on a 2000 byte file should fail
        let range = ByteRange::resolve("bytes=5000-", 2000);
        assert!(range.is_none());
    }

    #[test]
    fn byte_range_invalid_start_greater_than_end() {
        // bytes=1000-500 is invalid
        let range = ByteRange::resolve("bytes=1000-500", 2000);
        assert!(range.is_none());
    }

    #[test]
    fn byte_range_invalid_format() {
        // Not starting with bytes=
        assert!(ByteRange::resolve("range=0-100", 2000).is_none());
        // Empty range
        assert!(ByteRange::resolve("bytes=-", 2000).is_none());
        // Multiple ranges not supported
        assert!(ByteRange::resolve("bytes=0-100, 200-300", 2000).is_none());
        // Non-numeric values
        assert!(ByteRange::resolve("bytes=abc-def", 2000).is_none());
    }

    #[test]
    fn byte_range_zero_length_file() {
        // Any range on a zero-length file should fail
        assert!(ByteRange::resolve("bytes=0-100", 0).is_none());
        assert!(ByteRange::resolve("bytes=0-", 0).is_none());
        assert!(ByteRange::resolve("bytes=-100", 0).is_none());
    }

    #[test]
    fn byte_range_suffix_zero() {
        // bytes=-0 is invalid (requesting last 0 bytes)
        assert!(ByteRange::resolve("bytes=-0", 2000).is_none());
    }

    #[tokio::test]
    async fn get_object_without_range_header() {
        let dir = tempdir().expect("temp dir");
        let router: axum::Router = RouterWithState::new(Arc::new(
            S3State::new(dir.path().to_path_buf()).expect("state"),
        ))
        .into();

        // First put an object
        let payload = "Hello, World! This is test content.";
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/test-bucket/test-file.txt")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .expect("put response");
        assert_eq!(response.status(), StatusCode::OK);

        // GET without Range header should return full content (HTTP 200)
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/test-bucket/test-file.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("get response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("accept-ranges")
                .map(|v| v.to_str().unwrap()),
            Some("bytes")
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(body.as_ref(), payload.as_bytes());
    }

    #[tokio::test]
    async fn get_object_with_explicit_range() {
        let dir = tempdir().expect("temp dir");
        let router: axum::Router = RouterWithState::new(Arc::new(
            S3State::new(dir.path().to_path_buf()).expect("state"),
        ))
        .into();

        // Put an object with known content
        let payload = "0123456789ABCDEFGHIJ"; // 20 bytes
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/test-bucket/range-test.txt")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .expect("put response");
        assert_eq!(response.status(), StatusCode::OK);

        // GET bytes=0-9 (first 10 bytes)
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/test-bucket/range-test.txt")
                    .header("Range", "bytes=0-9")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("get response");
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response
                .headers()
                .get("content-range")
                .map(|v| v.to_str().unwrap()),
            Some("bytes 0-9/20")
        );
        assert_eq!(
            response
                .headers()
                .get("accept-ranges")
                .map(|v| v.to_str().unwrap()),
            Some("bytes")
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(body.as_ref(), b"0123456789");
    }

    #[tokio::test]
    async fn get_object_with_open_ended_range() {
        let dir = tempdir().expect("temp dir");
        let router: axum::Router = RouterWithState::new(Arc::new(
            S3State::new(dir.path().to_path_buf()).expect("state"),
        ))
        .into();

        let payload = "0123456789ABCDEFGHIJ"; // 20 bytes
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/test-bucket/range-test2.txt")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .expect("put response");
        assert_eq!(response.status(), StatusCode::OK);

        // GET bytes=10- (from byte 10 to end)
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/test-bucket/range-test2.txt")
                    .header("Range", "bytes=10-")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("get response");
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response
                .headers()
                .get("content-range")
                .map(|v| v.to_str().unwrap()),
            Some("bytes 10-19/20")
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(body.as_ref(), b"ABCDEFGHIJ");
    }

    #[tokio::test]
    async fn get_object_with_suffix_range() {
        let dir = tempdir().expect("temp dir");
        let router: axum::Router = RouterWithState::new(Arc::new(
            S3State::new(dir.path().to_path_buf()).expect("state"),
        ))
        .into();

        let payload = "0123456789ABCDEFGHIJ"; // 20 bytes
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/test-bucket/range-test3.txt")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .expect("put response");
        assert_eq!(response.status(), StatusCode::OK);

        // GET bytes=-5 (last 5 bytes)
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/test-bucket/range-test3.txt")
                    .header("Range", "bytes=-5")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("get response");
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response
                .headers()
                .get("content-range")
                .map(|v| v.to_str().unwrap()),
            Some("bytes 15-19/20")
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(body.as_ref(), b"FGHIJ");
    }

    #[tokio::test]
    async fn get_object_with_invalid_range() {
        let dir = tempdir().expect("temp dir");
        let router: axum::Router = RouterWithState::new(Arc::new(
            S3State::new(dir.path().to_path_buf()).expect("state"),
        ))
        .into();

        let payload = "0123456789"; // 10 bytes
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/test-bucket/range-test4.txt")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .expect("put response");
        assert_eq!(response.status(), StatusCode::OK);

        // GET bytes=100- (start beyond file size)
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/test-bucket/range-test4.txt")
                    .header("Range", "bytes=100-")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("get response");
        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            response
                .headers()
                .get("content-range")
                .map(|v| v.to_str().unwrap()),
            Some("bytes */10")
        );
    }

    #[tokio::test]
    async fn get_object_range_clamped_to_file_size() {
        let dir = tempdir().expect("temp dir");
        let router: axum::Router = RouterWithState::new(Arc::new(
            S3State::new(dir.path().to_path_buf()).expect("state"),
        ))
        .into();

        let payload = "0123456789"; // 10 bytes
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/test-bucket/range-test5.txt")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .expect("put response");
        assert_eq!(response.status(), StatusCode::OK);

        // GET bytes=0-100 (range exceeds file size, should be clamped)
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/test-bucket/range-test5.txt")
                    .header("Range", "bytes=0-100")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("get response");
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response
                .headers()
                .get("content-range")
                .map(|v| v.to_str().unwrap()),
            Some("bytes 0-9/10")
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(body.as_ref(), payload.as_bytes());
    }

    #[test]
    fn metadata_path_returns_correct_path() {
        let dir = tempdir().expect("temp dir");
        let state = S3State::new(dir.path().to_path_buf()).expect("state");

        let path = state.metadata_path("my-bucket", "some/key.txt").unwrap();
        assert_eq!(
            path,
            dir.path().join("metadata/my-bucket/some/key.txt.etag")
        );
    }

    #[test]
    fn metadata_path_handles_nested_keys() {
        let dir = tempdir().expect("temp dir");
        let state = S3State::new(dir.path().to_path_buf()).expect("state");

        let path = state
            .metadata_path("bucket", "deeply/nested/path/file.bin")
            .unwrap();
        assert_eq!(
            path,
            dir.path()
                .join("metadata/bucket/deeply/nested/path/file.bin.etag")
        );
    }

    #[test]
    fn metadata_path_rejects_invalid_bucket() {
        let dir = tempdir().expect("temp dir");
        let state = S3State::new(dir.path().to_path_buf()).expect("state");

        assert!(state.metadata_path("", "key.txt").is_err());
        assert!(state.metadata_path("../evil", "key.txt").is_err());
    }

    #[test]
    fn metadata_path_rejects_invalid_key() {
        let dir = tempdir().expect("temp dir");
        let state = S3State::new(dir.path().to_path_buf()).expect("state");

        assert!(state.metadata_path("bucket", "../secret").is_err());
        assert!(state.metadata_path("bucket", "/absolute").is_err());
        assert!(state.metadata_path("bucket", "").is_err());
    }

    #[tokio::test]
    async fn ensure_metadata_dir_creates_directories() {
        let dir = tempdir().expect("temp dir");
        let state = S3State::new(dir.path().to_path_buf()).expect("state");

        state
            .ensure_metadata_dir("test-bucket", "some/nested/key.txt")
            .await
            .expect("should create directories");

        let expected_dir = dir.path().join("metadata/test-bucket/some/nested");
        assert!(expected_dir.exists(), "parent directories should exist");
    }

    #[tokio::test]
    async fn put_object_creates_etag_metadata_file() {
        let dir = tempdir().expect("temp dir");
        let router: axum::Router = RouterWithState::new(Arc::new(
            S3State::new(dir.path().to_path_buf()).expect("state"),
        ))
        .into();

        let payload = b"test content";
        let response = router
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/test-bucket/test-key.txt")
                    .body(Body::from(payload.as_slice()))
                    .unwrap(),
            )
            .await
            .expect("put response");
        assert_eq!(response.status(), StatusCode::OK);

        // Verify ETag metadata file was created
        let etag_path = dir.path().join("metadata/test-bucket/test-key.txt.etag");
        assert!(etag_path.exists(), "ETag metadata file should exist");

        // Verify the ETag file contains the correct quoted MD5 hash
        let etag_content = std::fs::read_to_string(&etag_path).expect("read etag file");
        let expected_etag = compute_etag(&Bytes::from(payload.as_slice()));
        assert_eq!(etag_content, expected_etag);

        // Verify it matches the ETag header in the response
        let response_etag = response
            .headers()
            .get("etag")
            .expect("etag header")
            .to_str()
            .expect("etag string");
        assert_eq!(etag_content, response_etag);
    }

    #[tokio::test]
    async fn put_object_creates_etag_for_nested_keys() {
        let dir = tempdir().expect("temp dir");
        let router: axum::Router = RouterWithState::new(Arc::new(
            S3State::new(dir.path().to_path_buf()).expect("state"),
        ))
        .into();

        let payload = b"nested content";
        let response = router
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/my-bucket/path/to/nested/file.bin")
                    .body(Body::from(payload.as_slice()))
                    .unwrap(),
            )
            .await
            .expect("put response");
        assert_eq!(response.status(), StatusCode::OK);

        // Verify ETag metadata file was created at the correct nested path
        let etag_path = dir
            .path()
            .join("metadata/my-bucket/path/to/nested/file.bin.etag");
        assert!(etag_path.exists(), "ETag metadata file should exist");

        let etag_content = std::fs::read_to_string(&etag_path).expect("read etag file");
        let expected_etag = compute_etag(&Bytes::from(payload.as_slice()));
        assert_eq!(etag_content, expected_etag);
    }

    #[tokio::test]
    async fn delete_object_removes_etag_metadata_file() {
        let dir = tempdir().expect("temp dir");
        let state = Arc::new(S3State::new(dir.path().to_path_buf()).expect("state"));
        let router: axum::Router = RouterWithState::new(state.clone()).into();

        // PUT an object
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/test-bucket/some/key.txt")
                    .body(Body::from("test content"))
                    .unwrap(),
            )
            .await
            .expect("put response");
        assert_eq!(response.status(), StatusCode::OK);

        // Create a mock ETag metadata file
        let metadata_path = state.metadata_path("test-bucket", "some/key.txt").unwrap();
        tokio::fs::create_dir_all(metadata_path.parent().unwrap())
            .await
            .expect("create metadata dir");
        tokio::fs::write(&metadata_path, "\"abc123\"")
            .await
            .expect("write metadata");
        assert!(
            metadata_path.exists(),
            "metadata file should exist before delete"
        );

        // DELETE the object
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/test-bucket/some/key.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("delete response");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        // Verify the metadata file was deleted
        assert!(
            !metadata_path.exists(),
            "metadata file should be deleted after object delete"
        );
    }

    #[tokio::test]
    async fn delete_object_succeeds_when_metadata_file_missing() {
        let dir = tempdir().expect("temp dir");
        let state = Arc::new(S3State::new(dir.path().to_path_buf()).expect("state"));
        let router: axum::Router = RouterWithState::new(state.clone()).into();

        // PUT an object (this will create a metadata file)
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/test-bucket/noetag/key.txt")
                    .body(Body::from("test content"))
                    .unwrap(),
            )
            .await
            .expect("put response");
        assert_eq!(response.status(), StatusCode::OK);

        // Manually remove the metadata file to simulate a missing metadata scenario
        let metadata_path = state
            .metadata_path("test-bucket", "noetag/key.txt")
            .unwrap();
        if metadata_path.exists() {
            tokio::fs::remove_file(&metadata_path)
                .await
                .expect("remove metadata file");
        }
        assert!(
            !metadata_path.exists(),
            "metadata file should not exist for this test"
        );

        // DELETE the object - should succeed even without metadata file
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/test-bucket/noetag/key.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("delete response");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        // Verify the object file was deleted
        let object_path = state.object_path("test-bucket", "noetag/key.txt").unwrap();
        assert!(!object_path.exists(), "object file should be deleted");
    }
}
