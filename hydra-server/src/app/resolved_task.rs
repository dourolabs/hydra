use super::{AppState, BundleResolutionError, ResolvedBundle};
use crate::domain::sessions::Session;
use hydra_common::api::v1::sessions::{Bundle, MountItem};
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
    pub async fn resolve_task(
        &self,
        session: &Session,
    ) -> Result<ResolvedTask, TaskResolutionError> {
        let context = Self::resolve_context(session);
        let image = Self::resolve_image(session, &context, &self.config.job.default_image)?;

        Ok(ResolvedTask {
            context,
            image,
            env_vars: session.env_vars.clone(),
            secrets: session.secrets.clone(),
        })
    }

    /// Read the resolved bundle off `session.mount_spec`. The first
    /// `MountItem::Bundle` (typically `mounts[0]`) carries the lowered git
    /// source; callers lower service repos before calling `create_session`
    /// (see `agent_queue::resolve_mount_spec`), so this is a straight read.
    fn resolve_context(session: &Session) -> ResolvedBundle {
        let bundle = session
            .mount_spec
            .mounts
            .iter()
            .find_map(|m| match m {
                MountItem::Bundle { bundle, .. } => Some(bundle.clone()),
                _ => None,
            })
            .unwrap_or(Bundle::None);
        ResolvedBundle {
            bundle,
            default_image: None,
        }
    }

    fn resolve_image(
        session: &Session,
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

        if let Some(image) = image_from(session.image.as_ref())? {
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
}
