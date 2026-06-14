use super::{AppState, BundleResolutionError};
use crate::domain::sessions::Session;
use hydra_common::api::v1::sessions::{Bundle, MountItem};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ResolvedTask {
    pub bundle: Bundle,
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
        let bundle = Self::resolve_bundle(session);
        let image = Self::resolve_image(session, &self.config.job.default_image)?;

        Ok(ResolvedTask {
            bundle,
            image,
            env_vars: session.env_vars.clone(),
            secrets: session.secrets.clone(),
        })
    }

    /// Read the resolved bundle off `session.mount_spec`. The first
    /// `MountItem::Bundle` (typically `mounts[0]`) carries the lowered git
    /// source; callers lower service repos before calling `create_session`
    /// (see `agent_queue::resolve_mount_spec`), so this is a straight read.
    fn resolve_bundle(session: &Session) -> Bundle {
        session
            .mount_spec
            .mounts
            .iter()
            .find_map(|m| match m {
                MountItem::Bundle { bundle, .. } => Some(bundle.clone()),
                _ => None,
            })
            .unwrap_or(Bundle::None)
    }

    fn resolve_image(
        session: &Session,
        fallback_image: &str,
    ) -> Result<String, TaskResolutionError> {
        if let Some(image) = session.image.as_ref() {
            let trimmed = image.trim();
            if trimmed.is_empty() {
                return Err(TaskResolutionError::EmptyImage);
            }
            return Ok(trimmed.to_string());
        }

        let trimmed = fallback_image.trim();
        if trimmed.is_empty() {
            return Err(TaskResolutionError::MissingDefaultImage);
        }

        Ok(trimmed.to_string())
    }
}
