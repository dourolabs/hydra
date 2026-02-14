use super::{AppState, BundleResolutionError, ResolvedBundle};
use crate::domain::jobs::{Bundle, BundleSpec, Task};
use crate::store::StoreError;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ResolvedTask {
    pub context: ResolvedBundle,
    pub image: String,
    pub env_vars: HashMap<String, String>,
    pub secrets: Option<Vec<String>>,
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
    pub async fn resolve_task(&self, task: &Task) -> Result<ResolvedTask, TaskResolutionError> {
        let context = self.resolve_context(task).await?;
        let image = Self::resolve_image(task, &context, &self.config.job.default_image)?;

        Ok(ResolvedTask {
            context,
            image,
            env_vars: task.env_vars.clone(),
            secrets: task.secrets.clone(),
        })
    }

    async fn resolve_context(&self, task: &Task) -> Result<ResolvedBundle, BundleResolutionError> {
        self.resolve_bundle_spec(task.context.clone()).await
    }

    fn resolve_image(
        task: &Task,
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
                let repository_config =
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
                    .or_else(|| repository_config.default_branch.clone())
                    .unwrap_or_else(|| "main".to_string());

                Ok(ResolvedBundle {
                    bundle: Bundle::GitRepository {
                        url: repository_config.remote_url.clone(),
                        rev: resolved_rev,
                    },
                    default_image: repository_config.default_image.clone(),
                })
            }
        }
    }
}
