use crate::{AppState, state::ResolvedBundle, store::TaskStatusLog};
use axum::{
    Json, async_trait,
    extract::{FromRequestParts, Path, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use metis_common::{
    MetisId,
    artifacts::{Artifact, IssueDependency, IssueDependencyType},
    constants::{ENV_GH_TOKEN, ENV_METIS_ID},
    jobs::{CreateJobRequest, CreateJobResponse},
};
use serde_json::json;
use tracing::{error, info};

pub mod context;
pub mod kill;
pub mod logs;
pub mod status;

pub async fn create_job(
    State(state): State<AppState>,
    Json(payload): Json<CreateJobRequest>,
) -> Result<Json<CreateJobResponse>, ApiError> {
    info!("create_job invoked");
    let fallback_image = state.config.metis.worker_image.clone();

    let parent_ids: Vec<MetisId> = payload
        .parent_ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .collect();
    if parent_ids.iter().any(|id| id.is_empty()) {
        error!("create_job received an empty parent_id");
        return Err(ApiError::bad_request("parent_ids must not be empty"));
    }
    let parent_dependencies: Vec<IssueDependency> = parent_ids
        .iter()
        .map(|id| IssueDependency {
            dependency_type: IssueDependencyType::BlockedOn,
            issue_id: id.clone(),
        })
        .collect();

    // Generate a unique ID for the job
    let job_id: MetisId = uuid::Uuid::new_v4().hyphenated().to_string();

    let ResolvedBundle {
        bundle: context,
        github_token,
        default_image,
    } = state.service_state.resolve_bundle_spec(payload.context)?;
    let mut env_vars = payload.variables;
    if let Some(token) = github_token {
        env_vars.entry(ENV_GH_TOKEN.to_string()).or_insert(token);
    }
    env_vars.insert(ENV_METIS_ID.to_string(), job_id.clone());
    let image = resolve_image(payload.image, default_image, &fallback_image)?;

    // Store the task with context (status will be Pending)
    {
        let mut store = state.store.write().await;
        let artifact = Artifact::Session {
            program: payload.program.clone(),
            params: payload.params.clone(),
            context,
            image,
            env_vars,
            log: TaskStatusLog::default(),
            dependencies: parent_dependencies,
        };
        store
            .add_artifact_with_id(job_id.clone(), artifact, Utc::now())
            .await
            .map_err(|err| {
                error!(error = %err, job_id = %job_id, "failed to store task");
                ApiError::internal(anyhow::anyhow!("Failed to store task: {err}"))
            })?;
    }

    info!(
        job_id = %job_id,
        parent_count = parent_ids.len(),
        "task stored, will be started by background thread"
    );

    Ok(Json(CreateJobResponse { job_id }))
}

fn resolve_image(
    user_supplied: Option<String>,
    repo_default: Option<String>,
    fallback: &str,
) -> Result<String, ApiError> {
    if let Some(image) = user_supplied {
        let trimmed = image.trim();
        if trimmed.is_empty() {
            return Err(ApiError::bad_request("image must not be empty"));
        }
        return Ok(trimmed.to_string());
    }

    if let Some(default_image) = repo_default {
        let trimmed = default_image.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let trimmed = fallback.trim();
    if trimmed.is_empty() {
        return Err(ApiError::internal(anyhow::anyhow!(
            "default worker image must not be empty"
        )));
    }

    Ok(trimmed.to_string())
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    pub fn internal(error: impl Into<anyhow::Error>) -> Self {
        let err = error.into();
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: err.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(json!({ "error": self.message }));
        (self.status, body).into_response()
    }
}

#[derive(Debug, Clone)]
pub struct JobIdPath(pub MetisId);

#[async_trait]
impl<S> FromRequestParts<S> for JobIdPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(job_id) = Path::<MetisId>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        let trimmed = job_id.trim();
        if trimmed.is_empty() {
            return Err(ApiError::bad_request("job_id must not be empty"));
        }

        Ok(Self(trimmed.to_string()))
    }
}
