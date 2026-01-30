use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, head, put},
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
use tokio::io::AsyncWriteExt;
use tracing::warn;
use walkdir::WalkDir;

const S3_XML_NAMESPACE: &str = "http://s3.amazonaws.com/doc/2006-03-01/";
const DEFAULT_MAX_KEYS: usize = 1000;

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
        Ok(self.root_dir.join(bucket))
    }

    fn object_path(&self, bucket: &str, key: &str) -> Result<PathBuf, S3Error> {
        validate_bucket(bucket)?;
        let key = sanitize_key(key)?;
        Ok(self.root_dir.join(bucket).join(key))
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
            .route("/:bucket/*key", put(put_object))
            .route("/:bucket/*key", get(get_object))
            .route("/:bucket/*key", head(head_object))
            .route("/:bucket/*key", delete(delete_object))
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

#[derive(Debug)]
struct ListResult {
    entries: Vec<ObjectEntry>,
    is_truncated: bool,
    next_token: Option<String>,
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

async fn list_objects_v2(
    State(state): State<Arc<S3State>>,
    Path(bucket): Path<String>,
    Query(query): Query<ListObjectsQuery>,
) -> Response {
    let list_type = query.list_type.unwrap_or(2);
    if list_type != 2 {
        return s3_error(
            StatusCode::BAD_REQUEST,
            "InvalidRequest",
            "list-type=2 is required for ListObjectsV2",
        );
    }

    let prefix = match sanitize_prefix(query.prefix.as_deref().unwrap_or("")) {
        Ok(prefix) => prefix,
        Err(err) => return err.into_response(),
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
        Err(err) => return err.into_response(),
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
    let root_dir = state.root_dir.clone();
    let bucket_name = bucket.to_string();
    let prefix = prefix.to_string();
    let continuation = continuation_token.clone();
    let start_after = start_after.clone();

    let entries = tokio::task::spawn_blocking(move || {
        if !bucket_dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        for entry in WalkDir::new(&bucket_dir).follow_links(false) {
            let entry = entry.map_err(|err| S3Error::io("walking storage", err))?;
            if !entry.file_type().is_file() {
                continue;
            }

            let relative = entry
                .path()
                .strip_prefix(&root_dir)
                .map_err(|err| S3Error::io("computing object key", err))?;
            let key = relative.to_string_lossy().replace('\\', "/");
            let key = key
                .strip_prefix(&bucket_name)
                .and_then(|value| value.strip_prefix('/'))
                .unwrap_or(&key)
                .to_string();

            if !prefix.is_empty() && !key.starts_with(&prefix) {
                continue;
            }

            let metadata = entry
                .metadata()
                .map_err(|err| S3Error::io("reading metadata", err))?;
            let last_modified = metadata.modified().ok();
            let size = metadata.len();
            let etag = compute_etag_from_path(entry.path()).ok();

            entries.push(ObjectEntry {
                key,
                last_modified,
                size,
                etag,
            });
        }

        Ok(entries)
    })
    .await
    .map_err(|err| S3Error::io("listing objects", err))??;

    let mut entries = entries;
    entries.sort_by(|a, b| a.key.cmp(&b.key));

    let marker = continuation.or(start_after);
    if let Some(marker) = marker.as_ref() {
        entries.retain(|entry| entry.key.as_str() > marker.as_str());
    }

    let mut is_truncated = false;
    let mut next_token = None;
    if entries.len() > max_keys {
        is_truncated = true;
        let remainder = entries.split_off(max_keys);
        if let Some(last) = entries.last() {
            next_token = Some(last.key.clone());
        }
        drop(remainder);
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
    if body.is_empty() {
        warn!(bucket = %bucket, key = %key, "received empty PUT body");
    }

    let path = match state.object_path(&bucket, &key) {
        Ok(path) => path,
        Err(err) => return err.into_response(),
    };

    if let Some(parent) = path.parent() {
        if let Err(err) = tokio::fs::create_dir_all(parent).await {
            return S3Error::io("creating object directory", err).into_response();
        }
    }

    if let Err(err) = write_file(&path, &body).await {
        return S3Error::io("writing object", err).into_response();
    }

    let etag = compute_etag(&body);
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
    Path((bucket, key)): Path<(String, String)>,
) -> Response {
    let path = match state.object_path(&bucket, &key) {
        Ok(path) => path,
        Err(err) => return err.into_response(),
    };

    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return s3_error(StatusCode::NOT_FOUND, "NoSuchKey", "Object not found");
        }
        Err(err) => return S3Error::io("reading object", err).into_response(),
    };

    let metadata = match tokio::fs::metadata(&path).await {
        Ok(metadata) => metadata,
        Err(err) => return S3Error::io("reading metadata", err).into_response(),
    };

    let etag = compute_etag(&bytes);
    response_with_body(bytes, metadata.modified().ok(), metadata.len(), etag)
}

async fn head_object(
    State(state): State<Arc<S3State>>,
    Path((bucket, key)): Path<(String, String)>,
) -> Response {
    let path = match state.object_path(&bucket, &key) {
        Ok(path) => path,
        Err(err) => return err.into_response(),
    };

    let metadata = match tokio::fs::metadata(&path).await {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return s3_error(StatusCode::NOT_FOUND, "NoSuchKey", "Object not found");
        }
        Err(err) => return S3Error::io("reading metadata", err).into_response(),
    };

    let etag = match compute_etag_from_path(&path) {
        Ok(etag) => etag,
        Err(err) => return S3Error::io("reading object", err).into_response(),
    };
    response_with_body(Vec::new(), metadata.modified().ok(), metadata.len(), etag)
}

async fn delete_object(
    State(state): State<Arc<S3State>>,
    Path((bucket, key)): Path<(String, String)>,
) -> Response {
    let path = match state.object_path(&bucket, &key) {
        Ok(path) => path,
        Err(err) => return err.into_response(),
    };

    match tokio::fs::remove_file(&path).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return S3Error::io("deleting object", err).into_response(),
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

async fn write_file(path: &StdPath, body: &Bytes) -> Result<(), std::io::Error> {
    let mut file = tokio::fs::File::create(path).await?;
    file.write_all(body).await?;
    file.flush().await?;
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
    let bytes = std::fs::read(path)?;
    Ok(compute_etag(&bytes))
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
}
