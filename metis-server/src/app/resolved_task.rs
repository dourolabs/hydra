use super::{BundleResolutionError, ResolvedBundle, ServiceState};
use crate::domain::{
    issues::JobSettings,
    jobs::{Bundle, BundleSpec, Task},
};
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
        job_settings: Option<&JobSettings>,
    ) -> Result<ResolvedBundle, BundleResolutionError>;

    fn resolve_image(
        &self,
        resolved: &ResolvedBundle,
        fallback_image: &str,
        job_settings: Option<&JobSettings>,
    ) -> Result<String, TaskResolutionError>;

    fn resolve_env_vars(&self, resolved: &ResolvedBundle) -> HashMap<String, String>;

    async fn resolve(
        &self,
        service_state: &ServiceState,
        fallback_image: &str,
        job_settings: Option<&JobSettings>,
    ) -> Result<ResolvedTask, TaskResolutionError>;
}

#[async_trait]
impl TaskExt for Task {
    async fn resolve_context(
        &self,
        service_state: &ServiceState,
        job_settings: Option<&JobSettings>,
    ) -> Result<ResolvedBundle, BundleResolutionError> {
        let merged_settings = JobSettings::merge_refs(job_settings, self.job_settings.as_ref());
        let job_settings = merged_settings.as_ref();

        let mut resolved = service_state
            .resolve_bundle_spec(self.context.clone())
            .await?;

        if let Some(settings) = job_settings {
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
        }

        Ok(resolved)
    }

    fn resolve_image(
        &self,
        resolved: &ResolvedBundle,
        fallback_image: &str,
        job_settings: Option<&JobSettings>,
    ) -> Result<String, TaskResolutionError> {
        let merged_settings = JobSettings::merge_refs(job_settings, self.job_settings.as_ref());
        let job_settings = merged_settings.as_ref();

        if let Some(settings) = job_settings {
            if let Some(image) = &settings.image {
                let trimmed = image.trim();
                if trimmed.is_empty() {
                    return Err(TaskResolutionError::EmptyImage);
                }
                return Ok(trimmed.to_string());
            }
        }

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
        job_settings: Option<&JobSettings>,
    ) -> Result<ResolvedTask, TaskResolutionError> {
        let context = self.resolve_context(service_state, job_settings).await?;
        let image = self.resolve_image(&context, fallback_image, job_settings)?;
        let env_vars = self.resolve_env_vars(&context);

        Ok(ResolvedTask {
            context,
            image,
            env_vars,
        })
    }
}
