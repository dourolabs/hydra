use crate::domain::labels::Label;
use crate::store::{ReadOnlyStore, StoreError};
use metis_common::LabelId;
use metis_common::api::v1::labels::SearchLabelsQuery;
use thiserror::Error;

use super::AppState;

/// Default color palette for labels that don't specify a color.
const DEFAULT_COLORS: &[&str] = &[
    "#e74c3c", // red
    "#e67e22", // orange
    "#f1c40f", // yellow
    "#2ecc71", // green
    "#1abc9c", // teal
    "#3498db", // blue
    "#9b59b6", // purple
    "#e91e63", // pink
    "#795548", // brown
    "#607d8b", // blue grey
];

fn default_color_for_name(name: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let idx = (hasher.finish() as usize) % DEFAULT_COLORS.len();
    DEFAULT_COLORS[idx].to_string()
}

#[derive(Debug, Error)]
pub enum CreateLabelError {
    #[error("label name must not be empty")]
    EmptyName,
    #[error("a label named '{0}' already exists")]
    AlreadyExists(String),
    #[error("label store operation failed")]
    Store {
        #[source]
        source: StoreError,
    },
}

#[derive(Debug, Error)]
pub enum UpdateLabelError {
    #[error("label '{0}' not found")]
    NotFound(LabelId),
    #[error("label name must not be empty")]
    EmptyName,
    #[error("a label named '{0}' already exists")]
    AlreadyExists(String),
    #[error("label store operation failed")]
    Store {
        #[source]
        source: StoreError,
    },
}

impl AppState {
    pub async fn create_label(
        &self,
        name: String,
        color: Option<String>,
    ) -> Result<LabelId, CreateLabelError> {
        let name = name.trim().to_lowercase();
        if name.is_empty() {
            return Err(CreateLabelError::EmptyName);
        }

        let color = color.unwrap_or_else(|| default_color_for_name(&name));
        let label = Label::new(name, color);

        let label_id = self.store.add_label(label).await.map_err(|e| match e {
            StoreError::LabelAlreadyExists(name) => CreateLabelError::AlreadyExists(name),
            other => CreateLabelError::Store { source: other },
        })?;

        Ok(label_id)
    }

    pub async fn update_label(
        &self,
        label_id: &LabelId,
        name: String,
        color: Option<String>,
    ) -> Result<(), UpdateLabelError> {
        let existing = self.store.get_label(label_id).await.map_err(|e| match e {
            StoreError::LabelNotFound(id) => UpdateLabelError::NotFound(id),
            other => UpdateLabelError::Store { source: other },
        })?;

        let name = name.trim().to_lowercase();
        if name.is_empty() {
            return Err(UpdateLabelError::EmptyName);
        }

        let color = color.unwrap_or_else(|| existing.color.clone());
        let mut updated = existing;
        updated.name = name;
        updated.color = color;
        updated.updated_at = chrono::Utc::now();

        self.store
            .update_label(label_id, updated)
            .await
            .map_err(|e| match e {
                StoreError::LabelAlreadyExists(name) => UpdateLabelError::AlreadyExists(name),
                StoreError::LabelNotFound(id) => UpdateLabelError::NotFound(id),
                other => UpdateLabelError::Store { source: other },
            })?;

        Ok(())
    }

    pub async fn delete_label(&self, label_id: &LabelId) -> Result<(), StoreError> {
        self.store.delete_label(label_id).await
    }

    pub async fn get_label(&self, label_id: &LabelId) -> Result<Label, StoreError> {
        self.store.get_label(label_id).await
    }

    pub async fn list_labels(
        &self,
        query: &SearchLabelsQuery,
    ) -> Result<Vec<(LabelId, Label)>, StoreError> {
        self.store.list_labels(query).await
    }
}
