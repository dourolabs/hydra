use crate::{DocumentId, IssueId, MetisId, PatchId, TaskId, VersionNumber, Versioned};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde::{Deserialize, Serialize as SerdeSerialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, SerdeSerialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActivityObjectKind {
    Issue,
    Patch,
    Job,
    Document,
}

#[derive(Debug, Clone, PartialEq, SerdeSerialize, Deserialize)]
#[non_exhaustive]
pub struct FieldChange {
    pub path: String,
    pub before: Value,
    pub after: Value,
}

#[derive(Debug, Clone, PartialEq, SerdeSerialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActivityEvent {
    Created,
    Updated { changes: Vec<FieldChange> },
}

#[derive(Debug, Clone, PartialEq, SerdeSerialize, Deserialize)]
#[non_exhaustive]
pub struct ActivityLogEntry {
    pub object_id: MetisId,
    pub object_kind: ActivityObjectKind,
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    pub event: ActivityEvent,
    pub object: Value,
}

pub fn activity_log_for_issue_versions(
    issue_id: IssueId,
    versions: &[Versioned<crate::api::v1::issues::Issue>],
) -> Vec<ActivityLogEntry> {
    activity_log_from_versions(issue_id.into(), ActivityObjectKind::Issue, versions)
}

pub fn activity_log_for_patch_versions(
    patch_id: PatchId,
    versions: &[Versioned<crate::api::v1::patches::Patch>],
) -> Vec<ActivityLogEntry> {
    activity_log_from_versions(patch_id.into(), ActivityObjectKind::Patch, versions)
}

pub fn activity_log_for_document_versions(
    document_id: DocumentId,
    versions: &[Versioned<crate::api::v1::documents::Document>],
) -> Vec<ActivityLogEntry> {
    activity_log_from_versions(document_id.into(), ActivityObjectKind::Document, versions)
}

pub fn activity_log_for_job_versions<T: Serialize>(
    job_id: TaskId,
    versions: &[Versioned<T>],
) -> Vec<ActivityLogEntry> {
    activity_log_from_versions(job_id.into(), ActivityObjectKind::Job, versions)
}

pub fn activity_log_from_versions<T: Serialize>(
    object_id: MetisId,
    object_kind: ActivityObjectKind,
    versions: &[Versioned<T>],
) -> Vec<ActivityLogEntry> {
    if versions.is_empty() {
        return Vec::new();
    }

    let mut ordered: Vec<&Versioned<T>> = versions.iter().collect();
    ordered.sort_by(|a, b| {
        a.timestamp
            .cmp(&b.timestamp)
            .then_with(|| a.version.cmp(&b.version))
    });

    let mut entries = Vec::with_capacity(ordered.len());
    let mut previous_value: Option<Value> = None;

    for versioned in ordered {
        let current_value =
            serde_json::to_value(&versioned.item).expect("failed to serialize activity log item");
        let event = match &previous_value {
            None => ActivityEvent::Created,
            Some(previous) => ActivityEvent::Updated {
                changes: diff_json(previous, &current_value),
            },
        };

        entries.push(ActivityLogEntry {
            object_id: object_id.clone(),
            object_kind: object_kind.clone(),
            version: versioned.version,
            timestamp: versioned.timestamp,
            event,
            object: current_value.clone(),
        });

        previous_value = Some(current_value);
    }

    entries
}

fn diff_json(before: &Value, after: &Value) -> Vec<FieldChange> {
    let mut changes = Vec::new();
    diff_json_inner("", before, after, &mut changes);
    changes
}

fn diff_json_inner(path: &str, before: &Value, after: &Value, changes: &mut Vec<FieldChange>) {
    if before == after {
        return;
    }

    match (before, after) {
        (Value::Object(before_map), Value::Object(after_map)) => {
            let mut keys = BTreeSet::new();
            keys.extend(before_map.keys().cloned());
            keys.extend(after_map.keys().cloned());

            for key in keys {
                let before_value = before_map.get(&key).unwrap_or(&Value::Null);
                let after_value = after_map.get(&key).unwrap_or(&Value::Null);
                let next_path = join_path(path, &key);
                diff_json_inner(&next_path, before_value, after_value, changes);
            }
        }
        (Value::Array(before_values), Value::Array(after_values)) => {
            let max_len = before_values.len().max(after_values.len());
            for index in 0..max_len {
                let before_value = before_values.get(index).unwrap_or(&Value::Null);
                let after_value = after_values.get(index).unwrap_or(&Value::Null);
                let next_path = join_path(path, &index.to_string());
                diff_json_inner(&next_path, before_value, after_value, changes);
            }
        }
        _ => changes.push(FieldChange {
            path: normalize_path(path),
            before: before.clone(),
            after: after.clone(),
        }),
    }
}

fn join_path(parent: &str, segment: &str) -> String {
    if parent.is_empty() {
        format!("/{segment}")
    } else {
        format!("{parent}/{segment}")
    }
}

fn normalize_path(path: &str) -> String {
    if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ActivityEvent, ActivityObjectKind, activity_log_for_document_versions,
        activity_log_for_issue_versions, activity_log_for_patch_versions,
    };
    use crate::api::v1::documents::Document;
    use crate::api::v1::issues::{Issue, IssueStatus, IssueType};
    use crate::api::v1::patches::{Patch, PatchStatus};
    use crate::{DocumentId, IssueId, PatchId, RepoName, Versioned};
    use chrono::{TimeZone, Utc};

    #[test]
    fn activity_log_records_create_and_update_changes() {
        let issue_id = IssueId::new();
        let base_issue = Issue {
            issue_type: IssueType::Task,
            description: "Initial".to_string(),
            creator: "alice".into(),
            progress: String::new(),
            status: IssueStatus::Open,
            assignee: None,
            job_settings: Default::default(),
            todo_list: Vec::new(),
            dependencies: Vec::new(),
            patches: Vec::new(),
            deleted: false,
        };
        let updated_issue = Issue {
            description: "Updated".to_string(),
            ..base_issue.clone()
        };

        let versions = vec![
            Versioned::new(
                base_issue,
                1,
                Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap(),
            ),
            Versioned::new(
                updated_issue,
                2,
                Utc.with_ymd_and_hms(2024, 1, 2, 12, 0, 0).unwrap(),
            ),
        ];

        let log = activity_log_for_issue_versions(issue_id, &versions);
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].object_kind, ActivityObjectKind::Issue);
        assert!(matches!(log[0].event, ActivityEvent::Created));
        match &log[1].event {
            ActivityEvent::Updated { changes } => {
                assert_eq!(changes.len(), 1);
                let change = &changes[0];
                assert_eq!(change.path, "/description");
                assert_eq!(change.before, "Initial");
                assert_eq!(change.after, "Updated");
            }
            _ => panic!("expected updated event"),
        }
    }

    #[test]
    fn activity_log_orders_by_timestamp_then_version() {
        let patch_id = PatchId::new();
        let repo_name = RepoName::new("acme", "repo").unwrap();
        let patch_v1 = Patch {
            title: "v1".to_string(),
            description: "first".to_string(),
            diff: String::new(),
            status: PatchStatus::Open,
            is_automatic_backup: false,
            created_by: None,
            reviews: Vec::new(),
            service_repo_name: repo_name.clone(),
            github: None,
            deleted: false,
            branch_name: None,
            commit_range: None,
        };
        let patch_v2 = Patch {
            title: "v2".to_string(),
            ..patch_v1.clone()
        };

        let versions = vec![
            Versioned::new(
                patch_v2,
                2,
                Utc.with_ymd_and_hms(2024, 1, 2, 12, 0, 0).unwrap(),
            ),
            Versioned::new(
                patch_v1,
                1,
                Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap(),
            ),
        ];

        let log = activity_log_for_patch_versions(patch_id, &versions);
        assert_eq!(log.len(), 2);
        assert!(matches!(log[0].event, ActivityEvent::Created));
        assert!(matches!(log[1].event, ActivityEvent::Updated { .. }));
        assert!(log[0].timestamp < log[1].timestamp);
    }

    #[test]
    fn activity_log_captures_document_path_changes() {
        let document_id = DocumentId::new();
        let document_v1 = Document::new("Doc".to_string(), "body".to_string(), false);
        let document_v2 = Document {
            path: Some("docs/guide.md".to_string()),
            ..document_v1.clone()
        };
        let versions = vec![
            Versioned::new(
                document_v1,
                1,
                Utc.with_ymd_and_hms(2024, 1, 1, 8, 0, 0).unwrap(),
            ),
            Versioned::new(
                document_v2,
                2,
                Utc.with_ymd_and_hms(2024, 1, 1, 9, 0, 0).unwrap(),
            ),
        ];

        let log = activity_log_for_document_versions(document_id, &versions);
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].object_kind, ActivityObjectKind::Document);
        assert!(matches!(log[0].event, ActivityEvent::Created));
        match &log[1].event {
            ActivityEvent::Updated { changes } => {
                assert_eq!(changes.len(), 1);
                assert_eq!(changes[0].path, "/path");
                assert_eq!(changes[0].before, serde_json::Value::Null);
                assert_eq!(changes[0].after, "docs/guide.md");
            }
            other => panic!("expected updated event, got {other:?}"),
        }
    }
}
