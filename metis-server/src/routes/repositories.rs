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
            CreateRepositoryRequest, ListRepositoriesResponse, RepositoryAccessTokenResponse,
            UpdateRepositoryRequest, UpsertRepositoryResponse,
        },
    },
    repositories::GithubAppInstallationConfig,
};
use tracing::{error, info};

pub async fn list_repositories(
    State(state): State<AppState>,
) -> Result<Json<ListRepositoriesResponse>, ApiError> {
    info!("list_repositories invoked");
    let repositories = state.service_state.list_repository_info().await;
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
        .service_state
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
        .service_state
        .update_repository(name.clone(), config)
        .await
        .map_err(map_repository_error)?;

    info!(repository = %name, "update_repository completed");
    Ok(Json(UpsertRepositoryResponse::new(
        updated.without_secret(),
    )))
}

pub async fn get_repository_access_token(
    State(state): State<AppState>,
    Path((organization, repo)): Path<(String, String)>,
) -> Result<Json<RepositoryAccessTokenResponse>, ApiError> {
    let name =
        RepoName::new(organization, repo).map_err(|err| ApiError::bad_request(err.to_string()))?;
    info!(repository = %name, "get_repository_access_token invoked");

    let Some(repository) = state.service_state.repository(&name).await else {
        return Err(ApiError::not_found(format!(
            "repository '{name}' not found"
        )));
    };

    let token = crate::github_app::resolve_repository_access_token(&repository)
        .await
        .map_err(ApiError::internal)?;
    let Some(token) = token else {
        return Err(ApiError::bad_request(format!(
            "repository '{name}' has no GitHub App installation or token configured"
        )));
    };

    Ok(Json(RepositoryAccessTokenResponse::new(token)))
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
    config.github_token = config
        .github_token
        .and_then(|value| non_empty(&value).map(str::to_owned));
    config.github_app = normalize_github_app_config(config.github_app)?;

    Ok(config)
}

fn normalize_github_app_config(
    config: Option<GithubAppInstallationConfig>,
) -> Result<Option<GithubAppInstallationConfig>, ApiError> {
    let mut config = match config {
        Some(config) => config,
        None => return Ok(None),
    };

    if config.app_id == 0 {
        return Err(ApiError::bad_request(
            "github_app.app_id must be a positive integer",
        ));
    }
    if config.installation_id == 0 {
        return Err(ApiError::bad_request(
            "github_app.installation_id must be a positive integer",
        ));
    }

    config.private_key = config
        .private_key
        .and_then(|value| non_empty(&value).map(str::to_owned));
    config.key_path = config
        .key_path
        .and_then(|value| non_empty(&value).map(str::to_owned));

    if config.private_key.is_some() && config.key_path.is_some() {
        return Err(ApiError::bad_request(
            "github_app.private_key and github_app.key_path cannot both be set",
        ));
    }

    if config.private_key.is_none() && config.key_path.is_none() {
        return Err(ApiError::bad_request(
            "github_app.private_key or github_app.key_path must be set",
        ));
    }

    Ok(Some(config))
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
