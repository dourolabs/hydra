use gloo_net::http::Request;
use metis_common::{agents::ListAgentsResponse, jobs::ListJobsResponse};
use serde::de::DeserializeOwned;
use std::fmt;

use crate::config;

#[derive(Debug)]
pub enum ClientError {
    Request(String),
    Response(u16, String),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::Request(message) => write!(f, "request failed: {message}"),
            ClientError::Response(status, message) => {
                write!(f, "request failed with {status}: {message}")
            }
        }
    }
}

impl std::error::Error for ClientError {}

pub struct DashboardResponse {
    pub jobs: ListJobsResponse,
    pub agents: ListAgentsResponse,
}

pub async fn load_dashboard() -> Result<DashboardResponse, ClientError> {
    let jobs = get_json(api_url("/v1/jobs/")).await?;
    let agents = get_json(api_url("/v1/agents")).await?;

    Ok(DashboardResponse { jobs, agents })
}

async fn get_json<T: DeserializeOwned>(url: String) -> Result<T, ClientError> {
    let response = Request::get(&url)
        .send()
        .await
        .map_err(|err| ClientError::Request(err.to_string()))?;

    if !response.ok() {
        return Err(ClientError::Response(
            response.status(),
            response.status_text(),
        ));
    }

    response
        .json::<T>()
        .await
        .map_err(|err| ClientError::Request(err.to_string()))
}

fn api_url(path: &str) -> String {
    build_api_url(config::api_origin(), path)
}

fn build_api_url(origin: &str, path: &str) -> String {
    let base = origin.trim_end_matches('/');
    let path = path.strip_prefix('/').unwrap_or(path);

    if path.is_empty() {
        return base.to_string();
    }

    format!("{base}/{path}")
}

#[cfg(test)]
mod tests {
    use super::build_api_url;

    #[test]
    fn build_api_url_removes_duplicate_slashes() {
        let url = build_api_url("http://localhost:8080/", "/v1/jobs");
        assert_eq!(url, "http://localhost:8080/v1/jobs");
    }

    #[test]
    fn build_api_url_preserves_base_path() {
        let url = build_api_url("https://example.com/api", "v1/jobs");
        assert_eq!(url, "https://example.com/api/v1/jobs");
    }
}
