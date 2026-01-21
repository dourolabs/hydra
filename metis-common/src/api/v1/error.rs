use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::fmt::Display;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ApiErrorBody {
    pub error: String,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ApiError {
    status: StatusCode,
    body: ApiErrorBody,
}

impl ApiError {
    pub fn bad_request(message: impl Display) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn conflict(message: impl Display) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }

    pub fn not_found(message: impl Display) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub fn internal(message: impl Display) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn message(&self) -> &str {
        &self.body.error
    }

    fn new(status: StatusCode, message: impl Display) -> Self {
        Self {
            status,
            body: ApiErrorBody {
                error: message.to_string(),
            },
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}
