use gloo_net::http::Request;
use metis_common::{
    agents::ListAgentsResponse, constants::ENV_METIS_API_ORIGIN, jobs::ListJobsResponse,
};
use serde::de::DeserializeOwned;
use std::fmt;

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
    let jobs = get_json("/v1/jobs/").await?;
    let agents = get_json("/v1/agents").await?;

    Ok(DashboardResponse { jobs, agents })
}

async fn get_json<T: DeserializeOwned>(path: &str) -> Result<T, ClientError> {
    let url = build_api_url(path);
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

fn build_api_url(path: &str) -> String {
    match api_origin() {
        Some(origin) => {
            let origin = origin.trim_end_matches('/');
            let normalized_path = path.strip_prefix('/').unwrap_or(path);
            format!("{origin}/{normalized_path}")
        }
        None => path.to_string(),
    }
}

fn api_origin() -> Option<String> {
    let compile_time = option_env!("METIS_API_ORIGIN")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    if compile_time.is_some() {
        return compile_time;
    }

    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(value) = std::env::var(ENV_METIS_API_ORIGIN) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    None
}
