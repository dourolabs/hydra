use crate::domain::labels::Label;
use crate::store::{ReadOnlyStore, StoreError};
use metis_common::LabelId;
use metis_common::Rgb;
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

fn default_color_for_name(name: &str) -> Rgb {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let idx = (hasher.finish() as usize) % DEFAULT_COLORS.len();
    DEFAULT_COLORS[idx]
        .parse()
        .expect("DEFAULT_COLORS entries are valid hex colors")
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
        color: Option<Rgb>,
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
        color: Option<Rgb>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_state;

    #[tokio::test]
    async fn create_label_normalizes_name() {
        let state = test_state();
        let label_id = state
            .create_label("  My Label  ".to_string(), Some("#e74c3c".parse().unwrap()))
            .await
            .unwrap();

        let label = state.get_label(&label_id).await.unwrap();
        assert_eq!(label.name, "my label");
    }

    #[tokio::test]
    async fn create_label_rejects_empty_name() {
        let state = test_state();
        let err = state
            .create_label("   ".to_string(), None)
            .await
            .unwrap_err();
        assert!(matches!(err, CreateLabelError::EmptyName));
    }

    #[tokio::test]
    async fn create_label_assigns_default_color() {
        let state = test_state();
        let label_id = state.create_label("bug".to_string(), None).await.unwrap();

        let label = state.get_label(&label_id).await.unwrap();
        // Color should be one of the DEFAULT_COLORS palette entries
        assert!(
            DEFAULT_COLORS.contains(&label.color.as_ref()),
            "expected default palette color, got {}",
            label.color,
        );
    }

    #[tokio::test]
    async fn create_label_uses_explicit_color() {
        let state = test_state();
        let color: Rgb = "#abcdef".parse().unwrap();
        let label_id = state
            .create_label("bug".to_string(), Some(color.clone()))
            .await
            .unwrap();

        let label = state.get_label(&label_id).await.unwrap();
        assert_eq!(label.color, color);
    }

    #[tokio::test]
    async fn create_label_rejects_duplicate_normalized_name() {
        let state = test_state();
        state.create_label("Bug".to_string(), None).await.unwrap();

        let err = state
            .create_label("  bug  ".to_string(), None)
            .await
            .unwrap_err();
        assert!(matches!(err, CreateLabelError::AlreadyExists(_)));
    }

    #[tokio::test]
    async fn update_label_preserves_color_when_none() {
        let state = test_state();
        let color: Rgb = "#e74c3c".parse().unwrap();
        let label_id = state
            .create_label("bug".to_string(), Some(color.clone()))
            .await
            .unwrap();

        state
            .update_label(&label_id, "defect".to_string(), None)
            .await
            .unwrap();

        let label = state.get_label(&label_id).await.unwrap();
        assert_eq!(label.name, "defect");
        assert_eq!(label.color, color);
    }

    #[tokio::test]
    async fn update_label_rejects_empty_name() {
        let state = test_state();
        let label_id = state.create_label("bug".to_string(), None).await.unwrap();

        let err = state
            .update_label(&label_id, "  ".to_string(), None)
            .await
            .unwrap_err();
        assert!(matches!(err, UpdateLabelError::EmptyName));
    }

    #[tokio::test]
    async fn update_label_rejects_name_collision() {
        let state = test_state();
        state.create_label("bug".to_string(), None).await.unwrap();
        let feature_id = state
            .create_label("feature".to_string(), None)
            .await
            .unwrap();

        let err = state
            .update_label(&feature_id, "Bug".to_string(), None)
            .await
            .unwrap_err();
        assert!(matches!(err, UpdateLabelError::AlreadyExists(_)));
    }

    #[tokio::test]
    async fn delete_label_excludes_from_get_and_list() {
        let state = test_state();
        let label_id = state.create_label("bug".to_string(), None).await.unwrap();

        state.delete_label(&label_id).await.unwrap();

        // get_label returns not found
        let err = state.get_label(&label_id).await.unwrap_err();
        assert!(matches!(err, StoreError::LabelNotFound(_)));

        // list_labels excludes deleted by default
        let results = state
            .list_labels(&SearchLabelsQuery::default())
            .await
            .unwrap();
        assert!(results.is_empty());

        // list_labels with include_deleted returns soft-deleted labels
        let mut query = SearchLabelsQuery::default();
        query.include_deleted = Some(true);
        let results = state.list_labels(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].1.deleted);
    }

    #[tokio::test]
    async fn default_color_is_deterministic() {
        // Same name should always produce the same default color
        let color1 = default_color_for_name("bug");
        let color2 = default_color_for_name("bug");
        assert_eq!(color1, color2);

        // Different names can produce different colors
        let color3 = default_color_for_name("feature");
        // Just verify it's a valid palette color
        assert!(DEFAULT_COLORS.contains(&color3.as_ref()));
    }
}
