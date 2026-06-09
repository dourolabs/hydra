//! Per-project status configuration types.
//!
//! Defines the wire shapes for projects: a [`Project`] owns an ordered list
//! of [`StatusDefinition`]s. Each [`StatusDefinition`] declares display props
//! (label, color), dependency-graph semantics (`unblocks_parents`,
//! `unblocks_dependents`, `cascades_to_children`), and an optional
//! [`StatusOnEnter`] automation that fires when an issue transitions into
//! the status.

use crate::document_path::DocumentPath;
use crate::principal::Principal;
use crate::{Rgb, VersionNumber, api::v1::users::Username, ids::ProjectId};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

/// Maximum length for project / status keys. Keeps wire strings
/// bounded and well below any reasonable index limit.
pub const MAX_KEY_LENGTH: usize = 64;

/// Validation failure for newtyped string keys ([`ProjectKey`],
/// [`StatusKey`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    Empty,
    TooLong { actual: usize, max: usize },
    InvalidCharacters,
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::Empty => f.write_str("key must not be empty"),
            KeyError::TooLong { actual, max } => {
                write!(f, "key length {actual} exceeds maximum of {max}")
            }
            KeyError::InvalidCharacters => {
                f.write_str("key must contain only lowercase ASCII letters, digits, and '-'")
            }
        }
    }
}

impl std::error::Error for KeyError {}

fn validate_key(value: &str) -> Result<(), KeyError> {
    if value.is_empty() {
        return Err(KeyError::Empty);
    }
    if value.len() > MAX_KEY_LENGTH {
        return Err(KeyError::TooLong {
            actual: value.len(),
            max: MAX_KEY_LENGTH,
        });
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(KeyError::InvalidCharacters);
    }
    Ok(())
}

macro_rules! define_key_newtype {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        ///
        /// Charset: lowercase ASCII, digits, and `-`. Non-empty and at most
        /// [`MAX_KEY_LENGTH`] characters.
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
        #[serde(transparent)]
        #[cfg_attr(feature = "ts", derive(ts_rs::TS))]
        #[cfg_attr(feature = "ts", ts(export, type = "string"))]
        #[non_exhaustive]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn try_new(value: impl Into<String>) -> Result<Self, KeyError> {
                let value = value.into();
                validate_key(&value)?;
                Ok(Self(value))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = KeyError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::try_new(s)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let s = String::deserialize(deserializer)?;
                Self::try_new(s).map_err(serde::de::Error::custom)
            }
        }
    };
}

define_key_newtype!(ProjectKey, "Project slug, unique across projects.");
define_key_newtype!(
    StatusKey,
    "Status slug, unique within a project; the wire string for `Issue.status`."
);

/// Automation fired the moment an issue transitions INTO a status whose
/// [`StatusDefinition::on_enter`] is `Some`: when `assign_to` is set,
/// `issue.assignee` is replaced with that [`Principal`] (agent assignees
/// then flow through the existing assignee-driven spawn dispatcher); when
/// `attach_form` is set, `issue.form` is replaced wholesale with that
/// form (an issue holds at most one form at a time). `None` on either
/// field leaves the corresponding field untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct StatusOnEnter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assign_to: Option<Principal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach_form: Option<DocumentPath>,
}

impl StatusOnEnter {
    pub fn new(assign_to: Option<Principal>, attach_form: Option<DocumentPath>) -> Self {
        Self {
            assign_to,
            attach_form,
        }
    }
}

/// Declares one status within a project: display props, dependency
/// semantics, and an optional `on_enter` automation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct StatusDefinition {
    pub key: StatusKey,
    pub label: String,
    pub color: Rgb,
    pub unblocks_parents: bool,
    pub unblocks_dependents: bool,
    pub cascades_to_children: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_enter: Option<StatusOnEnter>,
    /// Doc-store path for the per-status prompt slice that gets concatenated
    /// into a session's `system_prompt` at create-time. `None` contributes
    /// an empty slice (typical for terminal statuses, which the spawn
    /// dispatcher skips anyway).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_path: Option<String>,
    /// When `true`, ready issues that land in this status spawn a
    /// conversation instead of a headless session.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub interactive: bool,
}

impl StatusDefinition {
    pub fn new(
        key: StatusKey,
        label: String,
        color: Rgb,
        unblocks_parents: bool,
        unblocks_dependents: bool,
        cascades_to_children: bool,
        on_enter: Option<StatusOnEnter>,
    ) -> Self {
        Self {
            key,
            label,
            color,
            unblocks_parents,
            unblocks_dependents,
            cascades_to_children,
            on_enter,
            prompt_path: None,
            interactive: false,
        }
    }
}

/// A project owns an ordered list of [`StatusDefinition`]s.
// No `Eq` derive: `priority` is `f64`. Use `PartialEq` for value equality.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct Project {
    pub key: ProjectKey,
    pub name: String,
    pub statuses: Vec<StatusDefinition>,
    pub creator: Username,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
    /// Doc-store path for the project-layer prompt slice that gets
    /// concatenated into a session's `system_prompt` at create-time.
    /// `None` contributes an empty slice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_path: Option<String>,
    /// Sort key for project ordering. Smaller values appear earlier.
    /// Default 0.0; drag-and-drop UI (follow-up) sets explicit values to
    /// reorder.
    #[serde(default)]
    pub priority: f64,
}

/// Validation failure for [`Project::validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectValidationError {
    DuplicateStatusKey(StatusKey),
    NoStatuses,
}

impl fmt::Display for ProjectValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectValidationError::DuplicateStatusKey(key) => {
                write!(f, "duplicate status key '{key}' in project")
            }
            ProjectValidationError::NoStatuses => {
                f.write_str("project must declare at least one status")
            }
        }
    }
}

impl std::error::Error for ProjectValidationError {}

impl Project {
    pub fn new(
        key: ProjectKey,
        name: String,
        statuses: Vec<StatusDefinition>,
        creator: Username,
        deleted: bool,
        priority: f64,
    ) -> Self {
        Self {
            key,
            name,
            statuses,
            creator,
            deleted,
            prompt_path: None,
            priority,
        }
    }

    /// Check structural invariants:
    /// - statuses is non-empty
    /// - all status keys are unique within the project
    pub fn validate(&self) -> Result<(), ProjectValidationError> {
        if self.statuses.is_empty() {
            return Err(ProjectValidationError::NoStatuses);
        }
        let mut seen: HashSet<&StatusKey> = HashSet::with_capacity(self.statuses.len());
        for status in &self.statuses {
            if !seen.insert(&status.key) {
                return Err(ProjectValidationError::DuplicateStatusKey(
                    status.key.clone(),
                ));
            }
        }
        Ok(())
    }

    /// Resolve a status by its [`StatusKey`] within this project. Returns
    /// `None` if no matching status is declared.
    pub fn find_status(&self, key: &StatusKey) -> Option<&StatusDefinition> {
        self.statuses.iter().find(|s| &s.key == key)
    }
}

/// Identifier for the project of an issue. `Default` is the synthesized
/// default project used for issues with no `project_id`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProjectScope {
    Default,
    Project(ProjectId),
}

/// Request body for `POST /v1/projects` and `PUT /v1/projects/:id`.
// No `Eq` derive: contains a `Project`, whose `priority` is `f64`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpsertProjectRequest {
    pub project: Project,
}

impl UpsertProjectRequest {
    pub fn new(project: Project) -> Self {
        Self { project }
    }
}

/// Response body for `POST /v1/projects` and `PUT /v1/projects/:id`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpsertProjectResponse {
    pub project_id: ProjectId,
    pub version: VersionNumber,
}

impl UpsertProjectResponse {
    pub fn new(project_id: ProjectId, version: VersionNumber) -> Self {
        Self {
            project_id,
            version,
        }
    }
}

/// Response body for `GET /v1/projects/:id`.
// No `Eq` derive: contains a `Project`, whose `priority` is `f64`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ProjectRecord {
    pub project_id: ProjectId,
    pub version: VersionNumber,
    pub project: Project,
}

impl ProjectRecord {
    pub fn new(project_id: ProjectId, version: VersionNumber, project: Project) -> Self {
        Self {
            project_id,
            version,
            project,
        }
    }
}

/// Response body for `GET /v1/projects`.
// No `Eq` derive: contains a `Project`, whose `priority` is `f64`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListProjectsResponse {
    pub projects: Vec<ProjectRecord>,
}

impl ListProjectsResponse {
    pub fn new(projects: Vec<ProjectRecord>) -> Self {
        Self { projects }
    }
}

/// Response body for `GET /v1/projects/:id/statuses`. Returned as an
/// ordered list matching the project's declaration order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ProjectStatusesResponse {
    pub statuses: Vec<StatusDefinition>,
}

impl ProjectStatusesResponse {
    pub fn new(statuses: Vec<StatusDefinition>) -> Self {
        Self { statuses }
    }
}

/// Path segment for `GET /v1/projects/:project_id_or_default/statuses`. Either
/// a real [`ProjectId`] or the literal `"default"` token addressing the
/// seeded default project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectIdOrDefault {
    Default,
    Id(ProjectId),
}

/// The wire token for the default project in `GET /v1/projects/:x/statuses`.
pub const DEFAULT_PROJECT_TOKEN: &str = "default";

impl fmt::Display for ProjectIdOrDefault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => f.write_str(DEFAULT_PROJECT_TOKEN),
            Self::Id(id) => fmt::Display::fmt(id, f),
        }
    }
}

impl FromStr for ProjectIdOrDefault {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == DEFAULT_PROJECT_TOKEN {
            return Ok(Self::Default);
        }
        ProjectId::try_from(s.to_string())
            .map(Self::Id)
            .map_err(|err| {
                format!(
                    "'{s}' is neither a valid project id nor the literal `{DEFAULT_PROJECT_TOKEN}`: {err}"
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(key: &str, label: &str) -> StatusDefinition {
        StatusDefinition::new(
            StatusKey::try_new(key).unwrap(),
            label.to_string(),
            "#abcdef".parse().unwrap(),
            false,
            false,
            false,
            None,
        )
    }

    fn project(statuses: Vec<StatusDefinition>) -> Project {
        Project::new(
            ProjectKey::try_new("eng").unwrap(),
            "Engineering".to_string(),
            statuses,
            Username::try_new("jayantk").unwrap(),
            false,
            0.0,
        )
    }

    #[test]
    fn key_try_new_accepts_lowercase_digits_dashes() {
        assert!(StatusKey::try_new("in-progress").is_ok());
        assert!(StatusKey::try_new("v2").is_ok());
        assert!(StatusKey::try_new("a").is_ok());
    }

    #[test]
    fn key_try_new_rejects_empty() {
        assert_eq!(StatusKey::try_new(""), Err(KeyError::Empty));
    }

    #[test]
    fn key_try_new_rejects_uppercase() {
        assert_eq!(
            StatusKey::try_new("InProgress"),
            Err(KeyError::InvalidCharacters)
        );
    }

    #[test]
    fn key_try_new_rejects_whitespace_and_slash() {
        assert_eq!(
            StatusKey::try_new("in progress"),
            Err(KeyError::InvalidCharacters)
        );
        assert_eq!(
            StatusKey::try_new("foo/bar"),
            Err(KeyError::InvalidCharacters)
        );
    }

    #[test]
    fn key_try_new_rejects_too_long() {
        let long = "a".repeat(MAX_KEY_LENGTH + 1);
        let err = StatusKey::try_new(long).unwrap_err();
        match err {
            KeyError::TooLong { actual, max } => {
                assert_eq!(actual, MAX_KEY_LENGTH + 1);
                assert_eq!(max, MAX_KEY_LENGTH);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn key_deserialize_validates() {
        let err = serde_json::from_str::<StatusKey>("\"InProgress\"").unwrap_err();
        assert!(err.to_string().contains("lowercase"));
    }

    #[test]
    fn project_serde_round_trip_with_full_status_shape() {
        let on_enter = StatusOnEnter::new(None, None);
        let mut def = status("in-progress", "In progress");
        def.unblocks_parents = false;
        def.unblocks_dependents = false;
        def.cascades_to_children = false;
        def.on_enter = Some(on_enter);
        let proj = project(vec![def]);

        let json = serde_json::to_string(&proj).unwrap();
        let parsed: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, proj);
    }

    #[test]
    fn status_definition_omits_on_enter_when_none() {
        let def = status("open", "Open");
        let json = serde_json::to_string(&def).unwrap();
        assert!(!json.contains("on_enter"));
    }

    #[test]
    fn status_definition_omits_prompt_path_when_none() {
        let def = status("open", "Open");
        let json = serde_json::to_string(&def).unwrap();
        assert!(
            !json.contains("prompt_path"),
            "prompt_path should be skipped when None; got {json}"
        );
    }

    #[test]
    fn status_definition_round_trips_prompt_path() {
        let mut def = status("open", "Open");
        def.prompt_path = Some("/projects/default/statuses/open.md".to_string());
        let json = serde_json::to_string(&def).unwrap();
        assert!(json.contains("\"prompt_path\""));
        let parsed: StatusDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.prompt_path.as_deref(),
            Some("/projects/default/statuses/open.md")
        );
    }

    #[test]
    fn project_omits_prompt_path_when_none() {
        let proj = project(vec![status("open", "Open")]);
        let json = serde_json::to_string(&proj).unwrap();
        assert!(
            !json.contains("prompt_path"),
            "prompt_path should be skipped when None; got {json}"
        );
    }

    #[test]
    fn project_round_trips_prompt_path() {
        let mut proj = project(vec![status("open", "Open")]);
        proj.prompt_path = Some("/projects/default/prompt.md".to_string());
        let json = serde_json::to_string(&proj).unwrap();
        assert!(json.contains("\"prompt_path\""));
        let parsed: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.prompt_path.as_deref(),
            Some("/projects/default/prompt.md")
        );
    }

    #[test]
    fn status_definition_omits_interactive_when_false() {
        let def = status("open", "Open");
        let json = serde_json::to_string(&def).unwrap();
        assert!(
            !json.contains("interactive"),
            "interactive should be skipped when false; got {json}"
        );
    }

    #[test]
    fn status_definition_round_trips_interactive_true() {
        let mut def = status("open", "Open");
        def.interactive = true;
        let json = serde_json::to_string(&def).unwrap();
        assert!(json.contains("\"interactive\":true"));
        let parsed: StatusDefinition = serde_json::from_str(&json).unwrap();
        assert!(parsed.interactive);
    }

    #[test]
    fn status_definition_defaults_interactive_when_field_absent() {
        // Older payloads (pre-PR1) have no `interactive`; deserialize to `false`.
        let legacy = serde_json::json!({
            "key": "open",
            "label": "Open",
            "color": "#abcdef",
            "unblocks_parents": false,
            "unblocks_dependents": false,
            "cascades_to_children": false,
        });
        let parsed: StatusDefinition = serde_json::from_value(legacy).unwrap();
        assert!(!parsed.interactive);
    }

    #[test]
    fn status_definition_round_trips_interactive_false_drops_field() {
        // Explicit `interactive: false` on the wire must re-serialize without the field.
        let wire = serde_json::json!({
            "key": "open",
            "label": "Open",
            "color": "#abcdef",
            "unblocks_parents": false,
            "unblocks_dependents": false,
            "cascades_to_children": false,
            "interactive": false,
        });
        let parsed: StatusDefinition = serde_json::from_value(wire).unwrap();
        assert!(!parsed.interactive);
        let reserialized = serde_json::to_string(&parsed).unwrap();
        assert!(
            !reserialized.contains("interactive"),
            "skip_serializing_if should drop `interactive: false`; got {reserialized}"
        );
    }

    #[test]
    fn project_deserializes_legacy_wire_payload_without_prompt_path() {
        // Old payload (pre-PR1) had no `prompt_path` on Project or StatusDefinition;
        // it must continue to parse to `None` so older clients don't break the wire.
        let legacy = serde_json::json!({
            "key": "eng",
            "name": "Engineering",
            "statuses": [
                {
                    "key": "open",
                    "label": "Open",
                    "color": "#abcdef",
                    "unblocks_parents": false,
                    "unblocks_dependents": false,
                    "cascades_to_children": false,
                }
            ],
            "creator": "jayantk",
        });
        let parsed: Project = serde_json::from_value(legacy).unwrap();
        assert!(parsed.prompt_path.is_none());
        assert!(parsed.statuses[0].prompt_path.is_none());
    }

    #[test]
    fn project_validate_accepts_well_formed() {
        let proj = project(vec![status("open", "Open"), status("closed", "Closed")]);
        proj.validate().unwrap();
    }

    #[test]
    fn project_validate_rejects_duplicate_status_keys() {
        let proj = project(vec![status("open", "Open"), status("open", "Open Again")]);
        let err = proj.validate().unwrap_err();
        assert!(matches!(err, ProjectValidationError::DuplicateStatusKey(_)));
    }

    #[test]
    fn project_validate_rejects_empty_status_list() {
        let proj = project(vec![]);
        let err = proj.validate().unwrap_err();
        assert!(matches!(err, ProjectValidationError::NoStatuses));
    }

    #[test]
    fn find_status_returns_matching_definition() {
        let proj = project(vec![status("open", "Open"), status("closed", "Closed")]);
        let key = StatusKey::try_new("closed").unwrap();
        let found = proj.find_status(&key).unwrap();
        assert_eq!(found.label, "Closed");
    }

    #[test]
    fn find_status_returns_none_for_unknown_key() {
        let proj = project(vec![status("open", "Open")]);
        let key = StatusKey::try_new("nope").unwrap();
        assert!(proj.find_status(&key).is_none());
    }

    #[test]
    fn project_serializes_priority() {
        let mut proj = project(vec![status("open", "Open")]);
        proj.priority = 1000.0;
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value.get("priority"), Some(&serde_json::json!(1000.0)));
    }

    #[test]
    fn project_deserializes_legacy_wire_payload_with_default_priority() {
        // Older payloads (pre-priority) had no `priority`; deserialize to `0.0`.
        let legacy = serde_json::json!({
            "key": "eng",
            "name": "Engineering",
            "statuses": [
                {
                    "key": "open",
                    "label": "Open",
                    "color": "#abcdef",
                    "unblocks_parents": false,
                    "unblocks_dependents": false,
                    "cascades_to_children": false,
                }
            ],
            "default_status_key": "open",
            "creator": "jayantk",
        });
        let parsed: Project = serde_json::from_value(legacy).unwrap();
        assert_eq!(parsed.priority, 0.0);
    }
}
