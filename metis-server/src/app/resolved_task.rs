use super::{BundleResolutionError, ResolvedBundle, ServiceState};
use crate::domain::jobs::Task;
use async_trait::async_trait;
use metis_common::constants::ENV_GH_TOKEN;
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

#[async_trait]
pub trait TaskExt {
    async fn resolve_context(
        &self,
        service_state: &ServiceState,
    ) -> Result<ResolvedBundle, BundleResolutionError>;

    fn resolve_image(
        &self,
        resolved: &ResolvedBundle,
        fallback_image: &str,
    ) -> Result<String, TaskResolutionError>;

    fn resolve_env_vars(&self, resolved: &ResolvedBundle) -> HashMap<String, String>;

    async fn resolve(
        &self,
        service_state: &ServiceState,
        fallback_image: &str,
    ) -> Result<ResolvedTask, TaskResolutionError>;
}

#[async_trait]
impl TaskExt for Task {
    async fn resolve_context(
        &self,
        service_state: &ServiceState,
    ) -> Result<ResolvedBundle, BundleResolutionError> {
        service_state
            .resolve_bundle_spec(self.context.clone())
            .await
    }

    fn resolve_image(
        &self,
        resolved: &ResolvedBundle,
        fallback_image: &str,
    ) -> Result<String, TaskResolutionError> {
        if let Some(image) = &self.image {
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

    fn resolve_env_vars(&self, resolved: &ResolvedBundle) -> HashMap<String, String> {
        let mut env_vars = self.env_vars.clone();
        if let Some(token) = &resolved.github_token {
            env_vars
                .entry(ENV_GH_TOKEN.to_string())
                .or_insert_with(|| token.clone());
        }
        env_vars
    }

    async fn resolve(
        &self,
        service_state: &ServiceState,
        fallback_image: &str,
    ) -> Result<ResolvedTask, TaskResolutionError> {
        let context = self.resolve_context(service_state).await?;
        let image = self.resolve_image(&context, fallback_image)?;
        let env_vars = self.resolve_env_vars(&context);

        Ok(ResolvedTask {
            context,
            image,
            env_vars,
        })
    }
}
