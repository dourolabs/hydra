use axum::{
    http::{HeaderValue, StatusCode},
    response::Response,
};
use std::time::SystemTime;

use super::ByteRange;

/// Creates a response with a body, setting appropriate headers for S3-compatible responses.
pub fn response_with_body(
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

/// Creates a streaming response for full content delivery.
pub fn streaming_response(
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

/// Creates a streaming response for partial content (HTTP 206).
pub fn streaming_partial_response(
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

/// Creates a 416 Range Not Satisfiable response.
pub fn range_not_satisfiable_response(total_len: u64) -> Response {
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
