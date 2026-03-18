use axum::{
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use tracing::warn;

use super::xml::xml_escape;

#[derive(Debug)]
pub struct S3Error {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl S3Error {
    pub fn bad_request(code: &'static str, message: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.to_string(),
        }
    }

    pub fn io(context: &'static str, err: impl std::fmt::Display) -> Self {
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

pub fn s3_error(status: StatusCode, code: &'static str, message: &str) -> Response {
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error>\n  <Code>{}</Code>\n  <Message>{}</Message>\n  <RequestId>hydra-s3</RequestId>\n</Error>\n",
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
