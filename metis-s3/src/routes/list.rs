use axum::{
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use tracing::{error, info};
use walkdir::WalkDir;

use crate::s3::S3State;
use crate::util::{
    DEFAULT_MAX_KEYS, ListObjectsQuery, ListResult, ObjectEntry, S3Error, render_list_response,
    s3_error, sanitize_prefix,
};

pub async fn list_objects_v2(
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

pub async fn list_objects(
    state: &S3State,
    bucket: &str,
    prefix: &str,
    continuation_token: Option<String>,
    start_after: Option<String>,
    max_keys: usize,
) -> Result<ListResult, S3Error> {
    let bucket_dir = state.bucket_dir(bucket)?;
    let metadata_dir = state.root_dir().join("metadata").join(bucket);
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

            // Read ETag from metadata cache (None if missing)
            let metadata_path = metadata_dir.join(format!("{key}.etag"));
            let etag = std::fs::read_to_string(&metadata_path).ok();

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
