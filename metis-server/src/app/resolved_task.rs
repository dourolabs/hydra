use super::{AppState, BundleResolutionError, ResolvedBundle, ServiceState};
use crate::domain::jobs::{Bundle, BundleSpec, Task};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ResolvedTask {
    #[allow(dead_code)]
    pub context: ResolvedBundle,
    pub image: String,
    pub env_vars: HashMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum TaskResolutionError {
    #[error(transparent)]
    Bundle(#[from] BundleResolutionError),
    #[error("image must not be empty")]
    EmptyImage,
    #[error("default worker image must not be empty")]
    MissingDefaultImage,
}

async fn resolve_context(
    task: &Task,
    service_state: &ServiceState,
) -> Result<ResolvedBundle, BundleResolutionError> {
    let mut resolved = service_state
        .resolve_bundle_spec(task.context.clone())
        .await?;

    let settings = &task.job_settings;
    if let Some(repo_name) = &settings.repo_name {
        resolved = service_state
            .resolve_bundle_spec(BundleSpec::ServiceRepository {
                name: repo_name.clone(),
                rev: settings.branch.clone(),
            })
            .await?;
    }

    if settings.remote_url.is_some() || settings.branch.is_some() {
        let url = settings
            .remote_url
            .clone()
            .or_else(|| match &resolved.bundle {
                Bundle::GitRepository { url, .. } => Some(url.clone()),
                Bundle::None => None,
            });
        let rev = settings
            .branch
            .clone()
            .or_else(|| match &resolved.bundle {
                Bundle::GitRepository { rev, .. } => Some(rev.clone()),
                Bundle::None => None,
            })
            .unwrap_or_else(|| "main".to_string());

        if let Some(url) = url {
            resolved.bundle = Bundle::GitRepository { url, rev };
        }
    }

    Ok(resolved)
}

fn resolve_image(
    task: &Task,
    resolved: &ResolvedBundle,
    fallback_image: &str,
) -> Result<String, TaskResolutionError> {
    if let Some(image) = &task.job_settings.image {
        let trimmed = image.trim();
        if trimmed.is_empty() {
            return Err(TaskResolutionError::EmptyImage);
        }
        return Ok(trimmed.to_string());
    }

    if let Some(image) = &task.image {
        let trimmed = image.trim();
        if trimmed.is_empty() {
            return Err(TaskResolutionError::EmptyImage);
        }
        return Ok(trimmed.to_string());
    }

    if let Some(default_image) = &resolved.default_image {
        let trimmed = default_image.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let trimmed = fallback_image.trim();
    if trimmed.is_empty() {
        return Err(TaskResolutionError::MissingDefaultImage);
    }

    Ok(trimmed.to_string())
}

fn resolve_env_vars(task: &Task, _resolved: &ResolvedBundle) -> HashMap<String, String> {
    task.env_vars.clone()
}

async fn resolve_task(
    task: &Task,
    service_state: &ServiceState,
    fallback_image: &str,
) -> Result<ResolvedTask, TaskResolutionError> {
    let context = resolve_context(task, service_state).await?;
    let image = resolve_image(task, &context, fallback_image)?;
    let env_vars = resolve_env_vars(task, &context);

    Ok(ResolvedTask {
        context,
        image,
        env_vars,
    })
}

impl AppState {
    pub async fn resolve_task(&self, task: &Task) -> Result<ResolvedTask, TaskResolutionError> {
        let fallback_image = &self.config.job.default_image;
        resolve_task(task, self.service_state.as_ref(), fallback_image).await
    }
}
