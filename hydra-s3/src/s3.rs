use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Response,
    routing::{delete, get, head, post, put},
};
use std::{path::PathBuf, sync::Arc};

use crate::routes::{
    abort_multipart_upload, complete_multipart_upload, create_multipart_upload, delete_object,
    get_object, head_object, list_objects_v2, put_object, upload_part,
};
use crate::util::{MultipartQuery, S3Error, s3_error, sanitize_key, validate_bucket};

#[derive(Clone, Debug)]
pub struct S3State {
    root_dir: PathBuf,
}

impl S3State {
    pub fn new(root_dir: PathBuf) -> Result<Self, std::io::Error> {
        std::fs::create_dir_all(&root_dir)?;
        Ok(Self { root_dir })
    }

    pub fn root_dir(&self) -> &PathBuf {
        &self.root_dir
    }

    pub fn bucket_dir(&self, bucket: &str) -> Result<PathBuf, S3Error> {
        validate_bucket(bucket)?;
        Ok(self.root_dir.join("buckets").join(bucket))
    }

    pub fn object_path(&self, bucket: &str, key: &str) -> Result<PathBuf, S3Error> {
        validate_bucket(bucket)?;
        let key = sanitize_key(key)?;
        Ok(self.root_dir.join("buckets").join(bucket).join(key))
    }

    pub fn multipart_dir(&self) -> PathBuf {
        self.root_dir.join("multipart")
    }

    pub fn upload_dir(&self, upload_id: &str) -> PathBuf {
        self.multipart_dir().join(upload_id)
    }

    pub fn part_path(&self, upload_id: &str, part_number: u32) -> PathBuf {
        self.upload_dir(upload_id)
            .join(format!("part-{part_number:05}"))
    }

    pub fn upload_metadata_path(&self, upload_id: &str) -> PathBuf {
        self.upload_dir(upload_id).join("metadata.json")
    }

    /// Returns the path for storing an object's ETag metadata.
    /// Path format: `{root_dir}/metadata/{bucket}/{key}.etag`
    pub fn metadata_path(&self, bucket: &str, key: &str) -> Result<PathBuf, S3Error> {
        validate_bucket(bucket)?;
        let key = sanitize_key(key)?;
        Ok(self
            .root_dir
            .join("metadata")
            .join(bucket)
            .join(format!("{key}.etag")))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::{
        ByteRange, compute_etag, extract_xml_value, parse_complete_multipart_request,
    };
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
        use crate::util::sanitize_prefix;
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
