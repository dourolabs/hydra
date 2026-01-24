use super::{AppState, BundleResolutionError, ResolvedBundle};
use crate::domain::{
    issues::JobSettings,
    jobs::{Bundle, BundleSpec, Task},
};
use crate::store::StoreError;
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

impl AppState {
    pub async fn resolve_task(
        &self,
        task: &Task,
        job_settings: Option<&JobSettings>,
    ) -> Result<ResolvedTask, TaskResolutionError> {
        let default_job_settings;
        let job_settings = match job_settings {
            Some(settings) => settings,
            None => {
                default_job_settings = JobSettings::default();
                &default_job_settings
            }
        };

        let context = self.resolve_context(task, job_settings).await?;
        let image =
            Self::resolve_image(task, job_settings, &context, &self.config.job.default_image)?;

        Ok(ResolvedTask {
            context,
            image,
            env_vars: task.env_vars.clone(),
        })
    }

    async fn resolve_context(
        &self,
        task: &Task,
        job_settings: &JobSettings,
    ) -> Result<ResolvedBundle, BundleResolutionError> {
        let mut resolved = self.resolve_bundle_spec(task.context.clone()).await?;

        if let Some(repo_name) = &job_settings.repo_name {
            resolved = self
                .resolve_bundle_spec(BundleSpec::ServiceRepository {
                    name: repo_name.clone(),
                    rev: job_settings.branch.clone(),
                })
                .await?;
        }

        let remote_url = job_settings
            .remote_url
            .clone()
            .or_else(|| match &resolved.bundle {
                Bundle::GitRepository { url, .. } => Some(url.clone()),
                Bundle::None => None,
            });
        let rev = job_settings
            .branch
            .clone()
            .or_else(|| match &resolved.bundle {
                Bundle::GitRepository { rev, .. } => Some(rev.clone()),
                Bundle::None => None,
            })
            .unwrap_or_else(|| "main".to_string());

        if let Some(url) = remote_url {
            resolved.bundle = Bundle::GitRepository { url, rev };
        }

        Ok(resolved)
    }

    fn resolve_image(
        task: &Task,
        job_settings: &JobSettings,
        resolved: &ResolvedBundle,
        fallback_image: &str,
    ) -> Result<String, TaskResolutionError> {
        let image_from = |value: Option<&String>| -> Result<Option<String>, TaskResolutionError> {
            match value {
                Some(image) => {
                    let trimmed = image.trim();
                    if trimmed.is_empty() {
                        Err(TaskResolutionError::EmptyImage)
                    } else {
                        Ok(Some(trimmed.to_string()))
                    }
                }
                None => Ok(None),
            }
        };

        if let Some(image) = image_from(job_settings.image.as_ref())? {
            return Ok(image);
        }

        if let Some(image) = image_from(task.image.as_ref())? {
            return Ok(image);
        }

        if let Some(image) = image_from(resolved.default_image.as_ref())? {
            return Ok(image);
        }

        let trimmed = fallback_image.trim();
        if trimmed.is_empty() {
            return Err(TaskResolutionError::MissingDefaultImage);
        }

        Ok(trimmed.to_string())
    }

    async fn resolve_bundle_spec(
        &self,
        spec: BundleSpec,
    ) -> Result<ResolvedBundle, BundleResolutionError> {
        match spec {
            BundleSpec::None => Ok(ResolvedBundle {
                bundle: Bundle::None,
                default_image: None,
            }),
            BundleSpec::GitRepository { url, rev } => Ok(ResolvedBundle {
                bundle: Bundle::GitRepository { url, rev },
                default_image: None,
            }),
            BundleSpec::ServiceRepository { name, rev } => {
                let repository =
                    self.repository_from_store(&name)
                        .await
                        .map_err(|source| match source {
                            StoreError::RepositoryNotFound(_) => {
                                BundleResolutionError::UnknownRepository(name.clone())
                            }
                            other => BundleResolutionError::RepositoryLookup {
                                repo_name: name.clone(),
                                source: other,
                            },
                        })?;

                let resolved_rev = rev
                    .or_else(|| repository.default_branch.clone())
                    .unwrap_or_else(|| "main".to_string());

                Ok(ResolvedBundle {
                    bundle: Bundle::GitRepository {
                        url: repository.remote_url.clone(),
                        rev: resolved_rev,
                    },
                    default_image: repository.default_image.clone(),
                })
            }
        }
    }
}
