use crate::{app::AppState, domain::users::Username, store::StoreError};
use axum::Json;
use axum::extract::{Path, State};
use hydra_common::api::v1::{self, ApiError};
use tracing::info;

pub async fn get_user(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<Json<v1::users::UserSummary>, ApiError> {
    info!(username = %username, "get_user invoked");

    let username = Username::from(username);
    let user = state.get_user(&username).await.map_err(|err| match err {
        StoreError::UserNotFound(name) => {
            info!(username = %name, "user not found");
            ApiError::not_found(format!("user '{name}' not found"))
        }
        other => {
            tracing::error!(error = %other, "failed to fetch user");
            ApiError::internal(format!("failed to fetch user: {other}"))
        }
    })?;

    let summary: v1::users::UserSummary = crate::domain::users::UserSummary::from(user).into();

    info!(username = %summary.username, "get_user completed");
    Ok(Json(summary))
}
