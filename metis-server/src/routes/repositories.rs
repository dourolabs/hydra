use crate::{
    app::{AppState, Repository, RepositoryError},
    config::non_empty,
};
use axum::{
    Json,
    extract::{Path, State},
};
use metis_common::{
    RepoName,
    api::v1::{
        ApiError,
        repositories::{
            CreateRepositoryRequest, GetRepositoryResponse, ListRepositoriesResponse,
            SetRepositorySummaryRequest, SetRepositorySummaryResponse, UpdateRepositoryRequest,
            UpsertRepositoryResponse,
        },
    },
    constants::MAX_REPOSITORY_SUMMARY_BYTES,
};
use tracing::{error, info};

pub async fn list_repositories(
    State(state): State<AppState>,
) -> Result<Json<ListRepositoriesResponse>, ApiError> {
    info!("list_repositories invoked");
    let repositories = state
        .list_repositories()
        .await
        .map_err(map_repository_error)?;
    let response = ListRepositoriesResponse::new(repositories);
    info!(
        repository_count = response.repositories.len(),
        "list_repositories completed"
    );
    Ok(Json(response))
}

pub async fn create_repository(
    State(state): State<AppState>,
    Json(payload): Json<CreateRepositoryRequest>,
) -> Result<Json<UpsertRepositoryResponse>, ApiError> {
    info!(repository = %payload.name, "create_repository invoked");
    let config = normalize_config(payload.repository)?;
    let created = state
        .create_repository(payload.name, config)
        .await
        .map_err(map_repository_error)?;

    info!(repository = %created.name, "create_repository completed");
    Ok(Json(UpsertRepositoryResponse::new(created)))
}

pub async fn update_repository(
    State(state): State<AppState>,
    Path((organization, repo)): Path<(String, String)>,
    Json(payload): Json<UpdateRepositoryRequest>,
) -> Result<Json<UpsertRepositoryResponse>, ApiError> {
    let name = parse_repo_name(organization, repo)?;
    info!(repository = %name, "update_repository invoked");

    let config = normalize_config(payload.repository)?;
    let updated = state
        .update_repository(name.clone(), config)
        .await
        .map_err(map_repository_error)?;

    info!(repository = %name, "update_repository completed");
    Ok(Json(UpsertRepositoryResponse::new(updated)))
}

pub async fn get_repository(
    State(state): State<AppState>,
    Path((organization, repo)): Path<(String, String)>,
) -> Result<Json<GetRepositoryResponse>, ApiError> {
    let name = parse_repo_name(organization, repo)?;
    info!(repository = %name, "get_repository invoked");
    let repository = state
        .get_repository(name.clone())
        .await
        .map_err(map_repository_error)?;
    info!(repository = %name, "get_repository completed");
    Ok(Json(GetRepositoryResponse::new(repository)))
}

pub async fn set_repository_summary(
    State(state): State<AppState>,
    Path((organization, repo)): Path<(String, String)>,
    Json(payload): Json<SetRepositorySummaryRequest>,
) -> Result<Json<SetRepositorySummaryResponse>, ApiError> {
    let name = parse_repo_name(organization, repo)?;
    info!(repository = %name, "set_repository_summary invoked");
    let summary = normalize_summary(payload.content_summary)?;
    let updated = state
        .set_repository_summary(name.clone(), summary)
        .await
        .map_err(map_repository_error)?;
    info!(repository = %name, "set_repository_summary completed");
    Ok(Json(SetRepositorySummaryResponse::new(updated)))
}

fn normalize_config(mut config: Repository) -> Result<Repository, ApiError> {
    config.remote_url = config.remote_url.trim().to_string();
    if config.remote_url.is_empty() {
        return Err(ApiError::bad_request("remote_url must not be empty"));
    }

    config.default_branch = config
        .default_branch
        .and_then(|value| non_empty(&value).map(str::to_owned));
    config.default_image = config
        .default_image
        .and_then(|value| non_empty(&value).map(str::to_owned));

    Ok(config)
}

fn map_repository_error(err: RepositoryError) -> ApiError {
    match err {
        RepositoryError::AlreadyExists(name) => {
            error!(repository = %name, "repository already exists");
            ApiError::conflict(format!("repository '{name}' already exists"))
        }
        RepositoryError::NotFound(name) => {
            error!(repository = %name, "repository not found");
            ApiError::not_found(format!("repository '{name}' not found"))
        }
        RepositoryError::Store { source } => {
            error!(error = %source, "repository store error");
            ApiError::internal("repository store error")
        }
        RepositoryError::Git { repo_name, source } => {
            error!(
                repository = %repo_name,
                error = %source,
                "failed to refresh repository"
            );
            ApiError::bad_request(format!(
                "failed to refresh repository '{repo_name}': {source}"
            ))
        }
    }
}

fn parse_repo_name(organization: String, repo: String) -> Result<RepoName, ApiError> {
    RepoName::new(organization, repo).map_err(|err| ApiError::bad_request(err.to_string()))
}

fn normalize_summary(content_summary: Option<String>) -> Result<Option<String>, ApiError> {
    match content_summary {
        Some(summary) => {
            if summary.trim().is_empty() {
                return Err(ApiError::bad_request(
                    "content summary must not be empty when provided",
                ));
            }
            if summary.len() > MAX_REPOSITORY_SUMMARY_BYTES {
                return Err(ApiError::bad_request(format!(
                    "content summary must be at most {MAX_REPOSITORY_SUMMARY_BYTES} bytes"
                )));
            }
            Ok(Some(summary))
        }
        None => Ok(None),
    }
}
