use super::{BundleResolutionError, ResolvedBundle, ServiceState};
use crate::domain::{issues::JobSettings, jobs::Task};
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
        let context = apply_job_settings_to_bundle_spec(self.context.clone(), job_settings);
        service_state.resolve_bundle_spec(context).await
    }

    fn resolve_image(
        &self,
        resolved: &ResolvedBundle,
        fallback_image: &str,
        job_settings: Option<&JobSettings>,
    ) -> Result<String, TaskResolutionError> {
        if let Some(image) = job_settings.and_then(|settings| settings.image.as_deref()) {
            let trimmed = image.trim();
            if trimmed.is_empty() {
                return Err(TaskResolutionError::EmptyImage);
            }
            return Ok(trimmed.to_string());
        }

        if let Some(image) = self.image.as_deref() {
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

fn apply_job_settings_to_bundle_spec(
    context: crate::domain::jobs::BundleSpec,
    job_settings: Option<&JobSettings>,
) -> crate::domain::jobs::BundleSpec {
    use crate::domain::jobs::BundleSpec;

    let Some(job_settings) = job_settings else {
        return context;
    };

    let existing_rev = match &context {
        BundleSpec::GitRepository { rev, .. } => Some(rev.clone()),
        BundleSpec::ServiceRepository { rev, .. } => rev.clone(),
        BundleSpec::None => None,
    };

    if let Some(remote_url) = job_settings.remote_url.as_ref() {
        let rev = job_settings
            .branch
            .clone()
            .or_else(|| existing_rev.clone())
            .unwrap_or_else(|| "main".to_string());
        return BundleSpec::GitRepository {
            url: remote_url.clone(),
            rev,
        };
    }

    if let Some(repo_name) = job_settings.repo_name.as_ref() {
        let rev = job_settings.branch.clone().or(existing_rev);
        return BundleSpec::ServiceRepository {
            name: repo_name.clone(),
            rev,
        };
    }

    if let Some(branch) = job_settings.branch.as_ref() {
        return match context {
            BundleSpec::GitRepository { url, .. } => BundleSpec::GitRepository {
                url,
                rev: branch.clone(),
            },
            BundleSpec::ServiceRepository { name, .. } => BundleSpec::ServiceRepository {
                name,
                rev: Some(branch.clone()),
            },
            other => other,
        };
    }

    context
}
