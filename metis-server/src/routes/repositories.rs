use crate::{
    app::{AppState, Repository, RepositoryError},
    config::non_empty,
    domain::actors::{Actor, ActorRef},
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use metis_common::{
    RepoName,
    api::v1::{
        ApiError,
        repositories::{
            CreateRepositoryRequest, DeleteRepositoryResponse, ListRepositoriesResponse,
            SearchRepositoriesQuery, UpdateRepositoryRequest, UpsertRepositoryResponse,
        },
    },
};
use tracing::{error, info};

pub async fn list_repositories(
    State(state): State<AppState>,
    Query(query): Query<SearchRepositoriesQuery>,
) -> Result<Json<ListRepositoriesResponse>, ApiError> {
    info!("list_repositories invoked");
    let repositories = state
        .list_repositories(&query)
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
    Extension(actor): Extension<Actor>,
    Json(payload): Json<CreateRepositoryRequest>,
) -> Result<Json<UpsertRepositoryResponse>, ApiError> {
    info!(repository = %payload.name, "create_repository invoked");
    let config = normalize_config(payload.repository)?;
    let created = state
        .create_repository(payload.name, config, ActorRef::from(&actor))
        .await
        .map_err(map_repository_error)?;

    info!(repository = %created.name, "create_repository completed");
    Ok(Json(UpsertRepositoryResponse::new(created)))
}

pub async fn update_repository(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((organization, repo)): Path<(String, String)>,
    Json(payload): Json<UpdateRepositoryRequest>,
) -> Result<Json<UpsertRepositoryResponse>, ApiError> {
    let name =
        RepoName::new(organization, repo).map_err(|err| ApiError::bad_request(err.to_string()))?;
    info!(repository = %name, "update_repository invoked");

    let config = normalize_config(payload.repository)?;
    let updated = state
        .update_repository(name.clone(), config, ActorRef::from(&actor))
        .await
        .map_err(map_repository_error)?;

    info!(repository = %name, "update_repository completed");
    Ok(Json(UpsertRepositoryResponse::new(updated)))
}

pub async fn delete_repository(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((organization, repo)): Path<(String, String)>,
) -> Result<Json<DeleteRepositoryResponse>, ApiError> {
    let name =
        RepoName::new(organization, repo).map_err(|err| ApiError::bad_request(err.to_string()))?;
    info!(repository = %name, "delete_repository invoked");

    let deleted = state
        .delete_repository(&name, ActorRef::from(&actor))
        .await
        .map_err(map_repository_error)?;

    info!(repository = %name, "delete_repository completed");
    Ok(Json(DeleteRepositoryResponse::new(deleted)))
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
