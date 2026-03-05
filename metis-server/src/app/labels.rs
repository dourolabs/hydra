use crate::domain::labels::Label;
use crate::store::{ReadOnlyStore, StoreError};
use metis_common::api::v1::labels::{LabelSummary, SearchLabelsQuery};
use metis_common::issues::IssueId;
use metis_common::{LabelId, MetisId, Rgb};
use std::collections::HashSet;
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

    pub async fn add_label_association(
        &self,
        label_id: &LabelId,
        object_id: &MetisId,
    ) -> Result<(), StoreError> {
        self.store.add_label_association(label_id, object_id).await
    }

    pub async fn remove_label_association(
        &self,
        label_id: &LabelId,
        object_id: &MetisId,
    ) -> Result<(), StoreError> {
        self.store
            .remove_label_association(label_id, object_id)
            .await
    }

    pub async fn get_labels_for_object(
        &self,
        object_id: &MetisId,
    ) -> Result<Vec<LabelSummary>, StoreError> {
        self.store.get_labels_for_object(object_id).await
    }

    pub async fn get_labels_for_objects(
        &self,
        object_ids: &[MetisId],
    ) -> Result<std::collections::HashMap<MetisId, Vec<LabelSummary>>, StoreError> {
        self.store.get_labels_for_objects(object_ids).await
    }

    /// Resolve a mix of label IDs and label names into a deduplicated set of LabelIds.
    /// Label names that don't exist are created automatically.
    pub async fn resolve_label_ids(
        &self,
        label_ids: Option<Vec<LabelId>>,
        label_names: Option<Vec<String>>,
    ) -> Result<Vec<LabelId>, CreateLabelError> {
        let mut resolved: Vec<LabelId> = label_ids.unwrap_or_default();

        if let Some(names) = label_names {
            for name in names {
                let name_lower = name.trim().to_lowercase();
                if name_lower.is_empty() {
                    continue;
                }
                match self.store.get_label_by_name(&name_lower).await {
                    Ok(Some((id, _))) => {
                        if !resolved.contains(&id) {
                            resolved.push(id);
                        }
                    }
                    Ok(None) => {
                        let id = self.create_label(name, None).await?;
                        resolved.push(id);
                    }
                    Err(e) => return Err(CreateLabelError::Store { source: e }),
                }
            }
        }

        Ok(resolved)
    }

    /// Recursively add a label to all transitive children of the given issue.
    pub async fn cascade_label_to_children(
        &self,
        label_id: &LabelId,
        issue_id: &IssueId,
    ) -> Result<(), StoreError> {
        let mut visited = HashSet::new();
        self.cascade_label_recursive(label_id, issue_id, &mut visited)
            .await
    }

    async fn cascade_label_recursive(
        &self,
        label_id: &LabelId,
        issue_id: &IssueId,
        visited: &mut HashSet<IssueId>,
    ) -> Result<(), StoreError> {
        if !visited.insert(issue_id.clone()) {
            return Ok(());
        }
        let children = self.store.get_issue_children(issue_id).await?;
        for child_id in children {
            let object_id = MetisId::from(child_id.clone());
            self.store
                .add_label_association(label_id, &object_id)
                .await?;
            Box::pin(self.cascade_label_recursive(label_id, &child_id, visited)).await?;
        }
        Ok(())
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

    #[tokio::test]
    async fn add_and_get_label_association() {
        let state = test_state();
        let label_id = state.create_label("bug".to_string(), None).await.unwrap();
        let object_id: MetisId = "i-testissue".parse().unwrap();

        state
            .add_label_association(&label_id, &object_id)
            .await
            .unwrap();

        let labels = state.get_labels_for_object(&object_id).await.unwrap();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].label_id, label_id);
        assert_eq!(labels[0].name, "bug");
    }

    #[tokio::test]
    async fn add_label_association_is_idempotent() {
        let state = test_state();
        let label_id = state.create_label("bug".to_string(), None).await.unwrap();
        let object_id: MetisId = "i-testissue".parse().unwrap();

        state
            .add_label_association(&label_id, &object_id)
            .await
            .unwrap();
        state
            .add_label_association(&label_id, &object_id)
            .await
            .unwrap();

        let labels = state.get_labels_for_object(&object_id).await.unwrap();
        assert_eq!(labels.len(), 1);
    }

    #[tokio::test]
    async fn remove_label_association() {
        let state = test_state();
        let label_id = state.create_label("bug".to_string(), None).await.unwrap();
        let object_id: MetisId = "i-testissue".parse().unwrap();

        state
            .add_label_association(&label_id, &object_id)
            .await
            .unwrap();
        state
            .remove_label_association(&label_id, &object_id)
            .await
            .unwrap();

        let labels = state.get_labels_for_object(&object_id).await.unwrap();
        assert!(labels.is_empty());
    }

    #[tokio::test]
    async fn remove_nonexistent_label_association_is_noop() {
        let state = test_state();
        let label_id = state.create_label("bug".to_string(), None).await.unwrap();
        let object_id: MetisId = "i-testissue".parse().unwrap();

        // Removing a non-existent association should not error
        state
            .remove_label_association(&label_id, &object_id)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn get_labels_for_object_excludes_deleted_labels() {
        let state = test_state();
        let label_id = state.create_label("bug".to_string(), None).await.unwrap();
        let object_id: MetisId = "i-testissue".parse().unwrap();

        state
            .add_label_association(&label_id, &object_id)
            .await
            .unwrap();
        state.delete_label(&label_id).await.unwrap();

        let labels = state.get_labels_for_object(&object_id).await.unwrap();
        assert!(labels.is_empty());
    }

    #[tokio::test]
    async fn get_labels_for_objects_batch() {
        let state = test_state();
        let label_a = state.create_label("bug".to_string(), None).await.unwrap();
        let label_b = state
            .create_label("feature".to_string(), None)
            .await
            .unwrap();

        let obj1: MetisId = "i-issueone".parse().unwrap();
        let obj2: MetisId = "i-issuetwo".parse().unwrap();
        let obj3: MetisId = "i-issuethree".parse().unwrap();

        state.add_label_association(&label_a, &obj1).await.unwrap();
        state.add_label_association(&label_b, &obj1).await.unwrap();
        state.add_label_association(&label_a, &obj2).await.unwrap();
        // obj3 has no labels

        let result = state
            .get_labels_for_objects(&[obj1.clone(), obj2.clone(), obj3.clone()])
            .await
            .unwrap();

        assert_eq!(result.get(&obj1).map(Vec::len), Some(2));
        assert_eq!(result.get(&obj2).map(Vec::len), Some(1));
        assert!(!result.contains_key(&obj3));
    }

    #[tokio::test]
    async fn resolve_label_ids_creates_missing_labels() {
        let state = test_state();
        let existing_id = state.create_label("bug".to_string(), None).await.unwrap();

        let resolved = state
            .resolve_label_ids(
                Some(vec![existing_id.clone()]),
                Some(vec!["feature".to_string(), "docs".to_string()]),
            )
            .await
            .unwrap();

        // Should have 3 labels: the existing one + 2 newly created
        assert_eq!(resolved.len(), 3);
        assert!(resolved.contains(&existing_id));

        // The new labels should now exist
        let feature = state.store.get_label_by_name("feature").await.unwrap();
        assert!(feature.is_some());
        let docs = state.store.get_label_by_name("docs").await.unwrap();
        assert!(docs.is_some());
    }

    #[tokio::test]
    async fn resolve_label_ids_deduplicates_names() {
        let state = test_state();

        let resolved = state
            .resolve_label_ids(None, Some(vec!["bug".to_string(), "Bug".to_string()]))
            .await
            .unwrap();

        // "Bug" normalizes to "bug" and is deduplicated (contains check)
        assert_eq!(resolved.len(), 1);
    }

    #[tokio::test]
    async fn resolve_label_ids_skips_empty_names() {
        let state = test_state();

        let resolved = state
            .resolve_label_ids(None, Some(vec!["  ".to_string(), "".to_string()]))
            .await
            .unwrap();

        assert!(resolved.is_empty());
    }

    #[tokio::test]
    async fn upsert_issue_with_label_ids_syncs_labels() {
        use crate::domain::actors::ActorRef;
        use crate::domain::issues::IssueStatus;
        use metis_common::api::v1 as api;

        let state = test_state();
        let label_a = state.create_label("bug".to_string(), None).await.unwrap();
        let label_b = state
            .create_label("feature".to_string(), None)
            .await
            .unwrap();

        let issue = crate::app::test_helpers::issue_with_status("test", IssueStatus::Open, vec![]);
        let mut request = api::issues::UpsertIssueRequest::new(issue.into(), None);
        request.label_ids = Some(vec![label_a.clone(), label_b.clone()]);

        let (issue_id, _) = state
            .upsert_issue(None, request, ActorRef::test())
            .await
            .unwrap();

        let object_id = MetisId::from(issue_id.clone());
        let labels = state.get_labels_for_object(&object_id).await.unwrap();
        assert_eq!(labels.len(), 2);

        // Update to only keep label_a
        let updated_issue =
            crate::app::test_helpers::issue_with_status("test updated", IssueStatus::Open, vec![]);
        let mut update_request = api::issues::UpsertIssueRequest::new(updated_issue.into(), None);
        update_request.label_ids = Some(vec![label_a.clone()]);

        state
            .upsert_issue(Some(issue_id.clone()), update_request, ActorRef::test())
            .await
            .unwrap();

        let labels = state.get_labels_for_object(&object_id).await.unwrap();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].label_id, label_a);
    }

    #[tokio::test]
    async fn upsert_issue_with_label_names_creates_and_assigns() {
        use crate::domain::actors::ActorRef;
        use crate::domain::issues::IssueStatus;
        use metis_common::api::v1 as api;

        let state = test_state();

        let issue = crate::app::test_helpers::issue_with_status("test", IssueStatus::Open, vec![]);
        let mut request = api::issues::UpsertIssueRequest::new(issue.into(), None);
        request.label_names = Some(vec!["new-label".to_string()]);

        let (issue_id, _) = state
            .upsert_issue(None, request, ActorRef::test())
            .await
            .unwrap();

        let object_id = MetisId::from(issue_id);
        let labels = state.get_labels_for_object(&object_id).await.unwrap();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].name, "new-label");
    }

    #[tokio::test]
    async fn cascade_label_to_single_level_children() {
        use crate::domain::actors::ActorRef;
        use crate::domain::issues::{IssueDependency, IssueDependencyType, IssueStatus};
        use metis_common::api::v1 as api;

        let state = test_state();

        // Create parent issue
        let parent =
            crate::app::test_helpers::issue_with_status("parent", IssueStatus::Open, vec![]);
        let (parent_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(parent.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Create child issue
        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child = crate::app::test_helpers::issue_with_status(
            "child",
            IssueStatus::Open,
            vec![child_dep],
        );
        let (child_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(child.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Create a label and assign to parent
        let label_id = state.create_label("bug".to_string(), None).await.unwrap();
        let parent_obj = MetisId::from(parent_id.clone());
        state
            .add_label_association(&label_id, &parent_obj)
            .await
            .unwrap();

        // Cascade to children
        state
            .cascade_label_to_children(&label_id, &parent_id)
            .await
            .unwrap();

        // Child should now have the label
        let child_obj = MetisId::from(child_id);
        let child_labels = state.get_labels_for_object(&child_obj).await.unwrap();
        assert_eq!(child_labels.len(), 1);
        assert_eq!(child_labels[0].label_id, label_id);
    }

    #[tokio::test]
    async fn cascade_label_to_multi_level_children() {
        use crate::domain::actors::ActorRef;
        use crate::domain::issues::{IssueDependency, IssueDependencyType, IssueStatus};
        use metis_common::api::v1 as api;

        let state = test_state();

        // Create grandparent → parent → child chain
        let grandparent =
            crate::app::test_helpers::issue_with_status("grandparent", IssueStatus::Open, vec![]);
        let (grandparent_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(grandparent.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let parent_dep = IssueDependency::new(IssueDependencyType::ChildOf, grandparent_id.clone());
        let parent = crate::app::test_helpers::issue_with_status(
            "parent",
            IssueStatus::Open,
            vec![parent_dep],
        );
        let (parent_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(parent.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child = crate::app::test_helpers::issue_with_status(
            "child",
            IssueStatus::Open,
            vec![child_dep],
        );
        let (child_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(child.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Create a label and cascade from grandparent
        let label_id = state
            .create_label("priority".to_string(), None)
            .await
            .unwrap();
        let gp_obj = MetisId::from(grandparent_id.clone());
        state
            .add_label_association(&label_id, &gp_obj)
            .await
            .unwrap();

        state
            .cascade_label_to_children(&label_id, &grandparent_id)
            .await
            .unwrap();

        // Both parent and child should have the label
        let parent_obj = MetisId::from(parent_id);
        let parent_labels = state.get_labels_for_object(&parent_obj).await.unwrap();
        assert_eq!(parent_labels.len(), 1);
        assert_eq!(parent_labels[0].label_id, label_id);

        let child_obj = MetisId::from(child_id);
        let child_labels = state.get_labels_for_object(&child_obj).await.unwrap();
        assert_eq!(child_labels.len(), 1);
        assert_eq!(child_labels[0].label_id, label_id);
    }

    #[tokio::test]
    async fn cascade_label_with_no_children_is_noop() {
        use crate::domain::actors::ActorRef;
        use crate::domain::issues::IssueStatus;
        use metis_common::api::v1 as api;

        let state = test_state();

        let issue = crate::app::test_helpers::issue_with_status("solo", IssueStatus::Open, vec![]);
        let (issue_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(issue.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let label_id = state.create_label("bug".to_string(), None).await.unwrap();
        let obj = MetisId::from(issue_id.clone());
        state.add_label_association(&label_id, &obj).await.unwrap();

        // Should succeed without error
        state
            .cascade_label_to_children(&label_id, &issue_id)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn child_issue_inherits_parent_labels_on_creation() {
        use crate::domain::actors::ActorRef;
        use crate::domain::issues::{IssueDependency, IssueDependencyType, IssueStatus};
        use metis_common::api::v1 as api;

        let state = test_state();

        // Create parent and assign labels
        let parent =
            crate::app::test_helpers::issue_with_status("parent", IssueStatus::Open, vec![]);
        let (parent_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(parent.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let label_a = state.create_label("bug".to_string(), None).await.unwrap();
        let label_b = state
            .create_label("priority".to_string(), None)
            .await
            .unwrap();
        let parent_obj = MetisId::from(parent_id.clone());
        state
            .add_label_association(&label_a, &parent_obj)
            .await
            .unwrap();
        state
            .add_label_association(&label_b, &parent_obj)
            .await
            .unwrap();

        // Create child with child-of dependency — should inherit both labels
        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child = crate::app::test_helpers::issue_with_status(
            "child",
            IssueStatus::Open,
            vec![child_dep],
        );
        let (child_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(child.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let child_obj = MetisId::from(child_id);
        let child_labels = state.get_labels_for_object(&child_obj).await.unwrap();
        assert_eq!(child_labels.len(), 2);

        let label_ids: HashSet<LabelId> = child_labels.iter().map(|l| l.label_id.clone()).collect();
        assert!(label_ids.contains(&label_a));
        assert!(label_ids.contains(&label_b));
    }

    #[tokio::test]
    async fn child_issue_no_inheritance_when_parent_has_no_labels() {
        use crate::domain::actors::ActorRef;
        use crate::domain::issues::{IssueDependency, IssueDependencyType, IssueStatus};
        use metis_common::api::v1 as api;

        let state = test_state();

        // Create parent with no labels
        let parent =
            crate::app::test_helpers::issue_with_status("parent", IssueStatus::Open, vec![]);
        let (parent_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(parent.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Create child
        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child = crate::app::test_helpers::issue_with_status(
            "child",
            IssueStatus::Open,
            vec![child_dep],
        );
        let (child_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(child.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let child_obj = MetisId::from(child_id);
        let child_labels = state.get_labels_for_object(&child_obj).await.unwrap();
        assert!(child_labels.is_empty());
    }

    #[tokio::test]
    async fn child_issue_inherits_and_merges_with_explicit_labels() {
        use crate::domain::actors::ActorRef;
        use crate::domain::issues::{IssueDependency, IssueDependencyType, IssueStatus};
        use metis_common::api::v1 as api;

        let state = test_state();

        // Create parent and assign a label
        let parent =
            crate::app::test_helpers::issue_with_status("parent", IssueStatus::Open, vec![]);
        let (parent_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(parent.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let inherited_label = state
            .create_label("inherited".to_string(), None)
            .await
            .unwrap();
        let parent_obj = MetisId::from(parent_id.clone());
        state
            .add_label_association(&inherited_label, &parent_obj)
            .await
            .unwrap();

        // Create a separate label that will be explicitly assigned to the child
        let explicit_label = state
            .create_label("explicit".to_string(), None)
            .await
            .unwrap();

        // Create child with child-of dependency AND explicit label
        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child = crate::app::test_helpers::issue_with_status(
            "child",
            IssueStatus::Open,
            vec![child_dep],
        );
        let mut request = api::issues::UpsertIssueRequest::new(child.into(), None);
        request.label_ids = Some(vec![explicit_label.clone()]);

        let (child_id, _) = state
            .upsert_issue(None, request, ActorRef::test())
            .await
            .unwrap();

        let child_obj = MetisId::from(child_id);
        let child_labels = state.get_labels_for_object(&child_obj).await.unwrap();
        // Should have both: the explicit label and the inherited label
        assert_eq!(child_labels.len(), 2);

        let label_ids: HashSet<LabelId> = child_labels.iter().map(|l| l.label_id.clone()).collect();
        assert!(label_ids.contains(&inherited_label));
        assert!(label_ids.contains(&explicit_label));
    }
}
