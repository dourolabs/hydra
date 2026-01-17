use crate::url::join_api_url;
use gloo_net::http::Request;
use metis_common::{
    agents::ListAgentsResponse, constants::ENV_METIS_DASHBOARD_API_URL, jobs::ListJobsResponse,
};
use serde::de::DeserializeOwned;
use std::fmt;
use web_sys::window;

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
    let url = api_url(path)?;
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

fn api_url(path: &str) -> Result<String, ClientError> {
    let base_url = api_base_url()?;
    Ok(join_api_url(&base_url, path))
}

fn api_base_url() -> Result<String, ClientError> {
    if let Some(url) = option_env!("METIS_DASHBOARD_API_URL") {
        return Ok(url.to_string());
    }

    let window = window().ok_or_else(|| {
        ClientError::Request("window unavailable for dashboard base URL".to_string())
    })?;
    let origin = window.location().origin().map_err(|err| {
        ClientError::Request(format!(
            "unable to read window origin; set {ENV_METIS_DASHBOARD_API_URL}: {err:?}"
        ))
    })?;

    Ok(origin)
}
