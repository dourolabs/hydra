use crate::{
    app::{AppState, RepositoryError, ServiceRepository, ServiceRepositoryConfig},
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
            CreateRepositoryRequest, ListRepositoriesResponse, UpdateRepositoryRequest,
            UpsertRepositoryResponse,
        },
    },
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
    let repository = build_repository(payload.name, payload.repository)?;
    let created = state
        .create_repository(repository)
        .await
        .map_err(map_repository_error)?;

    info!(repository = %created.name, "create_repository completed");
    Ok(Json(UpsertRepositoryResponse::new(
        created.without_secret(),
    )))
}

pub async fn update_repository(
    State(state): State<AppState>,
    Path((organization, repo)): Path<(String, String)>,
    Json(payload): Json<UpdateRepositoryRequest>,
) -> Result<Json<UpsertRepositoryResponse>, ApiError> {
    let name =
        RepoName::new(organization, repo).map_err(|err| ApiError::bad_request(err.to_string()))?;
    info!(repository = %name, "update_repository invoked");

    let config = normalize_config(payload.repository)?;
    let updated = state
        .update_repository(name.clone(), config)
        .await
        .map_err(map_repository_error)?;

    info!(repository = %name, "update_repository completed");
    Ok(Json(UpsertRepositoryResponse::new(
        updated.without_secret(),
    )))
}

fn build_repository(
    name: RepoName,
    config: ServiceRepositoryConfig,
) -> Result<ServiceRepository, ApiError> {
    let normalized = normalize_config(config)?;
    Ok(ServiceRepository::from((name, normalized)))
}

fn normalize_config(
    mut config: ServiceRepositoryConfig,
) -> Result<ServiceRepositoryConfig, ApiError> {
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
