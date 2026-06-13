//! Per-project status configuration types.
//!
//! Defines the wire shapes for projects: a [`Project`] owns an ordered list
//! of [`StatusDefinition`]s. Each [`StatusDefinition`] declares display props
//! (label, color), dependency-graph semantics (`unblocks_parents`,
//! `unblocks_dependents`, `cascades_to_children`), and an optional
//! [`StatusOnEnter`] automation that fires when an issue transitions into
//! the status.

use crate::api::v1::issues::SessionSettings;
use crate::document_path::DocumentPath;
use crate::ids::HydraId;
use crate::principal::Principal;
use crate::{Rgb, VersionNumber, api::v1::users::Username, ids::ProjectId};
use serde::{Deserialize, Deserializer, Serialize};
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
    ReservedHydraIdShape,
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
            KeyError::ReservedHydraIdShape => f.write_str(
                "keys cannot use a single-letter prefix followed by `-`; reserved for HydraIds",
            ),
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
    if HydraId::is_id_or_reserved_shape(value) {
        return Err(KeyError::ReservedHydraIdShape);
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
/// form (an issue holds at most one form at a time); when
/// `clear_assignee` is `true`, `issue.assignee` is unset; when
/// `teardown_work` is `true`, this status is marked as a "teardown" status
/// — entering it kills any `Created`/`Pending`/`Running` sessions attached
/// to the issue AND closes any non-`Closed` conversations spawned from
/// the issue. The teardown effects also fire unconditionally on issue
/// deletion (regardless of this flag), but the flag is the canonical
/// "this is a teardown status" marker that gates the status-entry path.
/// `assign_to` and `clear_assignee` are mutually exclusive — set both
/// and [`StatusOnEnter::validate`] rejects the config; `teardown_work`
/// is independent of either. `None` (or `false`) on a field leaves the
/// corresponding field untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct StatusOnEnter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assign_to: Option<Principal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach_form: Option<DocumentPath>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clear_assignee: bool,
    #[serde(
        default,
        alias = "kill_sessions",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub teardown_work: bool,
}

impl StatusOnEnter {
    pub fn new(assign_to: Option<Principal>, attach_form: Option<DocumentPath>) -> Self {
        Self {
            assign_to,
            attach_form,
            clear_assignee: false,
            teardown_work: false,
        }
    }

    /// Reject configurations that contradict themselves. Today the only
    /// rule: `assign_to` (set the assignee to X) and `clear_assignee`
    /// (unset the assignee) cannot both be set.
    pub fn validate(&self) -> Result<(), String> {
        if self.clear_assignee && self.assign_to.is_some() {
            return Err("on_enter cannot set both assign_to and clear_assignee".to_string());
        }
        Ok(())
    }
}

/// Declares one status within a project: display props, dependency
/// semantics, and an optional `on_enter` automation.
// No `Eq` derive: `position` is `f64`. Use `PartialEq` for value equality.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// When `true`, ready issues that land in this status do NOT spawn an
    /// agent session (headless or interactive). Lets a project mark
    /// statuses as "tracked but inert" without changing the dependency
    /// semantics encoded by `unblocks_parents` / `unblocks_dependents`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub suppress_sessions: bool,
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
    /// When `Some(N)`, issues that have sat in this status for at least
    /// `N` seconds get auto-archived by a periodic worker. `None` (the
    /// default) leaves the feature off for the status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_archive_after_seconds: Option<i64>,
    /// When `Some(N)`, at most `N` active sessions (counting both
    /// interactive and headless, across all agents) may be associated
    /// with issues currently in this status. New spawns block until the
    /// active count drops below the cap. `None` (the default) leaves the
    /// cap off. Existing sessions above a freshly-lowered cap are not
    /// torn down — enforcement is only on new spawns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_simultaneous_sessions: Option<u32>,
    /// Sort key for status ordering within a project. Smaller values
    /// appear earlier. Default 0.0; drag-and-drop UI sets explicit
    /// values to reorder. Mirrors the existing [`Project::priority`]
    /// pattern.
    #[serde(default)]
    pub position: f64,
    /// Per-status overrides for the [`SessionSettings`] applied when
    /// spawning sessions for issues in this status. Merges with
    /// `Issue.session_settings` (issue-level wins) and the global
    /// defaults during spawn — see
    /// `SessionSettings::merge` and `apply_session_settings_defaults`.
    #[serde(default, skip_serializing_if = "SessionSettings::is_default")]
    pub session_settings: SessionSettings,
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
            suppress_sessions: false,
            on_enter,
            prompt_path: None,
            interactive: false,
            auto_archive_after_seconds: None,
            max_simultaneous_sessions: None,
            position: 0.0,
            session_settings: SessionSettings::default(),
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
    pub archived: bool,
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

impl Project {
    pub fn new(
        key: ProjectKey,
        name: String,
        statuses: Vec<StatusDefinition>,
        creator: Username,
        archived: bool,
        priority: f64,
    ) -> Self {
        Self {
            key,
            name,
            statuses,
            creator,
            archived,
            prompt_path: None,
            priority,
        }
    }

    /// Resolve a status by its [`StatusKey`] within this project. Returns
    /// `None` if no matching status is declared.
    pub fn find_status(&self, key: &StatusKey) -> Option<&StatusDefinition> {
        self.statuses.iter().find(|s| &s.key == key)
    }
}

/// Request body for `POST /v1/projects` and `PUT /v1/projects/:project_ref`.
///
/// Carries only project-level fields. Statuses are managed independently
/// via `POST/PUT/DELETE /v1/projects/:project_ref/statuses[/:status_key]`.
// No `Eq` derive: `priority` is `f64`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpsertProjectRequest {
    pub key: ProjectKey,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_path: Option<String>,
    #[serde(default)]
    pub priority: f64,
}

impl UpsertProjectRequest {
    pub fn new(key: ProjectKey, name: String) -> Self {
        Self {
            key,
            name,
            prompt_path: None,
            priority: 0.0,
        }
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

/// Response body for `GET /v1/projects/:project_ref/statuses`. Returned
/// as an ordered list matching the project's declaration order.
// No `Eq` derive: `StatusDefinition.position` is `f64`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

/// Response body for `POST /v1/projects/:project_ref/statuses` and
/// `PUT /v1/projects/:project_ref/statuses/:status_key`. Echoes the
/// status as the server persisted it (the inserted row's display props
/// after any server-side defaulting); `version` is the project's new
/// version number.
// No `Eq` derive: `StatusDefinition.position` is `f64`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpsertProjectStatusResponse {
    pub project_id: ProjectId,
    pub version: VersionNumber,
    pub status: StatusDefinition,
}

impl UpsertProjectStatusResponse {
    pub fn new(project_id: ProjectId, version: VersionNumber, status: StatusDefinition) -> Self {
        Self {
            project_id,
            version,
            status,
        }
    }
}

/// A project-addressing path segment. Accepted by every external
/// project surface (HTTP routes + CLI) — server-side code resolves
/// down to a [`ProjectId`] before invoking the store layer.
///
/// `FromStr` / `Deserialize` dispatch on
/// [`HydraId::is_id_or_reserved_shape`]: matching shapes parse as
/// [`ProjectId`] (the existing `j-…` form), anything else parses as a
/// [`ProjectKey`]. The construction-time enforcement that `ProjectKey`
/// cannot share a HydraId shape (see [`KeyError::ReservedHydraIdShape`])
/// guarantees the two forms are mutually exclusive by construction.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(untagged)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum ProjectRef {
    Id(ProjectId),
    Key(ProjectKey),
}

impl ProjectRef {
    /// Returns the wire string for this reference — the same value
    /// `Display` produces.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Id(id) => id.as_ref(),
            Self::Key(key) => key.as_str(),
        }
    }
}

impl fmt::Display for ProjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(id) => fmt::Display::fmt(id, f),
            Self::Key(key) => fmt::Display::fmt(key, f),
        }
    }
}

impl FromStr for ProjectRef {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if HydraId::is_id_or_reserved_shape(s) {
            ProjectId::try_from(s.to_string())
                .map(Self::Id)
                .map_err(|err| format!("'{s}' is not a valid project id: {err}"))
        } else {
            ProjectKey::try_new(s)
                .map(Self::Key)
                .map_err(|err| format!("'{s}' is not a valid project key: {err}"))
        }
    }
}

impl<'de> Deserialize<'de> for ProjectRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        ProjectRef::from_str(&value).map_err(serde::de::Error::custom)
    }
}

impl From<ProjectId> for ProjectRef {
    fn from(id: ProjectId) -> Self {
        Self::Id(id)
    }
}

impl From<ProjectKey> for ProjectRef {
    fn from(key: ProjectKey) -> Self {
        Self::Key(key)
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
    fn key_try_new_rejects_reserved_hydra_id_shape() {
        for shape in ["i-foo", "p-bar", "s-todo", "j-foo", "x-foobar", "a-"] {
            assert_eq!(
                StatusKey::try_new(shape),
                Err(KeyError::ReservedHydraIdShape),
                "expected `{shape}` to be rejected as reserved shape via try_new"
            );
            assert_eq!(
                ProjectKey::try_new(shape),
                Err(KeyError::ReservedHydraIdShape),
                "expected `{shape}` to be rejected as reserved shape via try_new (ProjectKey)"
            );
        }
    }

    #[test]
    fn key_from_str_rejects_reserved_hydra_id_shape() {
        let err: KeyError = "i-progress".parse::<StatusKey>().unwrap_err();
        assert_eq!(err, KeyError::ReservedHydraIdShape);
        let err: KeyError = "j-foo".parse::<ProjectKey>().unwrap_err();
        assert_eq!(err, KeyError::ReservedHydraIdShape);
    }

    #[test]
    fn key_deserialize_rejects_reserved_hydra_id_shape() {
        let err = serde_json::from_str::<StatusKey>("\"i-progress\"").unwrap_err();
        assert!(
            err.to_string().contains("reserved for HydraIds"),
            "expected reserved-shape mention; got: {err}"
        );
        let err = serde_json::from_str::<ProjectKey>("\"j-foo\"").unwrap_err();
        assert!(
            err.to_string().contains("reserved for HydraIds"),
            "expected reserved-shape mention; got: {err}"
        );
    }

    #[test]
    fn key_try_new_accepts_safe_lookalikes() {
        for safe in [
            "renamed-x-foo",
            "ab-foo",
            "default",
            "open",
            "in-progress",
            "eng-high",
        ] {
            assert!(
                StatusKey::try_new(safe).is_ok(),
                "expected `{safe}` to pass StatusKey::try_new"
            );
            assert!(
                ProjectKey::try_new(safe).is_ok(),
                "expected `{safe}` to pass ProjectKey::try_new"
            );
        }
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
    fn status_on_enter_validate_accepts_neither_field_set() {
        assert!(StatusOnEnter::new(None, None).validate().is_ok());
    }

    #[test]
    fn status_on_enter_validate_accepts_assign_to_alone() {
        let agent = crate::api::v1::agents::AgentName::try_new("reviewer").unwrap();
        let on_enter = StatusOnEnter::new(Some(Principal::Agent { name: agent }), None);
        assert!(on_enter.validate().is_ok());
    }

    #[test]
    fn status_on_enter_validate_accepts_clear_assignee_alone() {
        let mut on_enter = StatusOnEnter::new(None, None);
        on_enter.clear_assignee = true;
        assert!(on_enter.validate().is_ok());
    }

    #[test]
    fn status_on_enter_validate_rejects_assign_to_with_clear_assignee() {
        let agent = crate::api::v1::agents::AgentName::try_new("reviewer").unwrap();
        let mut on_enter = StatusOnEnter::new(Some(Principal::Agent { name: agent }), None);
        on_enter.clear_assignee = true;
        let err = on_enter.validate().unwrap_err();
        assert!(
            err.contains("assign_to") && err.contains("clear_assignee"),
            "validation error must name both fields; got: {err}"
        );
    }

    #[test]
    fn status_on_enter_omits_clear_assignee_when_false() {
        let on_enter = StatusOnEnter::new(None, None);
        let json = serde_json::to_string(&on_enter).unwrap();
        assert!(
            !json.contains("clear_assignee"),
            "clear_assignee should be skipped when false; got {json}"
        );
    }

    #[test]
    fn status_on_enter_round_trips_clear_assignee_true() {
        let mut on_enter = StatusOnEnter::new(None, None);
        on_enter.clear_assignee = true;
        let json = serde_json::to_string(&on_enter).unwrap();
        assert!(json.contains("\"clear_assignee\":true"));
        let parsed: StatusOnEnter = serde_json::from_str(&json).unwrap();
        assert!(parsed.clear_assignee);
    }

    #[test]
    fn status_on_enter_defaults_clear_assignee_when_field_absent() {
        // Legacy payloads (pre-this-PR) have no `clear_assignee`; they
        // must continue to parse with the field defaulted to `false`.
        let legacy = serde_json::json!({});
        let parsed: StatusOnEnter = serde_json::from_value(legacy).unwrap();
        assert!(!parsed.clear_assignee);
    }

    #[test]
    fn status_on_enter_validate_accepts_teardown_work_alone() {
        let mut on_enter = StatusOnEnter::new(None, None);
        on_enter.teardown_work = true;
        assert!(on_enter.validate().is_ok());
    }

    #[test]
    fn status_on_enter_validate_accepts_teardown_work_with_assign_to() {
        let agent = crate::api::v1::agents::AgentName::try_new("reviewer").unwrap();
        let mut on_enter = StatusOnEnter::new(Some(Principal::Agent { name: agent }), None);
        on_enter.teardown_work = true;
        assert!(on_enter.validate().is_ok());
    }

    #[test]
    fn status_on_enter_validate_accepts_teardown_work_with_clear_assignee() {
        let mut on_enter = StatusOnEnter::new(None, None);
        on_enter.clear_assignee = true;
        on_enter.teardown_work = true;
        assert!(on_enter.validate().is_ok());
    }

    #[test]
    fn status_on_enter_validate_accepts_teardown_work_with_attach_form() {
        let form: DocumentPath = "/projects/default/forms/triage".parse().unwrap();
        let mut on_enter = StatusOnEnter::new(None, Some(form));
        on_enter.teardown_work = true;
        assert!(on_enter.validate().is_ok());
    }

    #[test]
    fn status_on_enter_omits_teardown_work_when_false() {
        let on_enter = StatusOnEnter::new(None, None);
        let json = serde_json::to_string(&on_enter).unwrap();
        assert!(
            !json.contains("teardown_work"),
            "teardown_work should be skipped when false; got {json}"
        );
    }

    #[test]
    fn status_on_enter_round_trips_teardown_work_true() {
        let mut on_enter = StatusOnEnter::new(None, None);
        on_enter.teardown_work = true;
        let json = serde_json::to_string(&on_enter).unwrap();
        assert!(json.contains("\"teardown_work\":true"));
        let parsed: StatusOnEnter = serde_json::from_str(&json).unwrap();
        assert!(parsed.teardown_work);
    }

    #[test]
    fn status_on_enter_defaults_teardown_work_when_field_absent() {
        let legacy = serde_json::json!({});
        let parsed: StatusOnEnter = serde_json::from_value(legacy).unwrap();
        assert!(!parsed.teardown_work);
    }

    #[test]
    fn status_on_enter_deserializes_legacy_kill_sessions_alias() {
        let legacy = serde_json::json!({ "kill_sessions": true });
        let parsed: StatusOnEnter = serde_json::from_value(legacy).unwrap();
        assert!(
            parsed.teardown_work,
            "the `kill_sessions` serde alias should map to `teardown_work`"
        );
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
    fn status_definition_omits_auto_archive_after_seconds_when_none() {
        let def = status("open", "Open");
        let json = serde_json::to_string(&def).unwrap();
        assert!(
            !json.contains("auto_archive_after_seconds"),
            "auto_archive_after_seconds should be skipped when None; got {json}"
        );
    }

    #[test]
    fn status_definition_round_trips_auto_archive_after_seconds() {
        let mut def = status("open", "Open");
        def.auto_archive_after_seconds = Some(1_209_600);
        let json = serde_json::to_string(&def).unwrap();
        assert!(json.contains("\"auto_archive_after_seconds\":1209600"));
        let parsed: StatusDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.auto_archive_after_seconds, Some(1_209_600));
    }

    #[test]
    fn status_definition_defaults_auto_archive_after_seconds_when_field_absent() {
        // Legacy payload (pre-this-PR) has no `auto_archive_after_seconds`;
        // it must deserialize to `None`.
        let legacy = serde_json::json!({
            "key": "open",
            "label": "Open",
            "color": "#abcdef",
            "unblocks_parents": false,
            "unblocks_dependents": false,
            "cascades_to_children": false,
        });
        let parsed: StatusDefinition = serde_json::from_value(legacy).unwrap();
        assert!(parsed.auto_archive_after_seconds.is_none());
    }

    #[test]
    fn status_definition_omits_max_simultaneous_sessions_when_none() {
        let def = status("open", "Open");
        let json = serde_json::to_string(&def).unwrap();
        assert!(
            !json.contains("max_simultaneous_sessions"),
            "max_simultaneous_sessions should be skipped when None; got {json}"
        );
    }

    #[test]
    fn status_definition_round_trips_max_simultaneous_sessions() {
        let mut def = status("open", "Open");
        def.max_simultaneous_sessions = Some(5);
        let json = serde_json::to_string(&def).unwrap();
        assert!(json.contains("\"max_simultaneous_sessions\":5"));
        let parsed: StatusDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.max_simultaneous_sessions, Some(5));
    }

    #[test]
    fn status_definition_defaults_max_simultaneous_sessions_when_field_absent() {
        let legacy = serde_json::json!({
            "key": "open",
            "label": "Open",
            "color": "#abcdef",
            "unblocks_parents": false,
            "unblocks_dependents": false,
            "cascades_to_children": false,
        });
        let parsed: StatusDefinition = serde_json::from_value(legacy).unwrap();
        assert!(parsed.max_simultaneous_sessions.is_none());
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
    fn project_ref_parses_id_shape() {
        let parsed: ProjectRef = "j-abcdef".parse().unwrap();
        match parsed {
            ProjectRef::Id(id) => assert_eq!(id.as_ref(), "j-abcdef"),
            ProjectRef::Key(_) => panic!("expected ProjectRef::Id"),
        }
    }

    #[test]
    fn project_ref_parses_key_shape() {
        let parsed: ProjectRef = "engineering".parse().unwrap();
        match parsed {
            ProjectRef::Key(key) => assert_eq!(key.as_str(), "engineering"),
            ProjectRef::Id(_) => panic!("expected ProjectRef::Key"),
        }
    }

    #[test]
    fn project_ref_parses_default_token_as_key() {
        // `"default"` no longer needs a dedicated variant: it is a valid
        // ProjectKey and resolves through the key-lookup path to the
        // seeded default project's id.
        let parsed: ProjectRef = "default".parse().unwrap();
        match parsed {
            ProjectRef::Key(key) => assert_eq!(key.as_str(), "default"),
            ProjectRef::Id(_) => panic!("expected ProjectRef::Key for \"default\""),
        }
    }

    #[test]
    fn project_ref_rejects_invalid_key_shape() {
        // Uppercase characters aren't a key, and `Foo` isn't id-shaped,
        // so the parse fails at the key validation step.
        let err = "Foo".parse::<ProjectRef>().unwrap_err();
        assert!(err.contains("not a valid project key"), "got: {err}");
    }

    #[test]
    fn project_ref_rejects_id_shape_with_invalid_suffix() {
        // `j-` has the id shape (single letter + `-`) but is too short
        // to be a valid `ProjectId`. The dispatcher routes to the id
        // branch so the error surfaces as an id error, not a key error.
        let err = "j-".parse::<ProjectRef>().unwrap_err();
        assert!(err.contains("not a valid project id"), "got: {err}");
    }

    #[test]
    fn project_ref_deserialize_dispatches_to_key() {
        let parsed: ProjectRef = serde_json::from_str("\"engineering\"").unwrap();
        assert!(matches!(parsed, ProjectRef::Key(_)));
    }

    #[test]
    fn project_ref_deserialize_dispatches_to_id() {
        let parsed: ProjectRef = serde_json::from_str("\"j-abcdef\"").unwrap();
        assert!(matches!(parsed, ProjectRef::Id(_)));
    }

    #[test]
    fn project_ref_display_round_trips_both_shapes() {
        let id_ref: ProjectRef = ProjectId::try_from("j-abcdef".to_string()).unwrap().into();
        assert_eq!(id_ref.to_string(), "j-abcdef");

        let key_ref: ProjectRef = ProjectKey::try_new("engineering").unwrap().into();
        assert_eq!(key_ref.to_string(), "engineering");
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

    #[test]
    fn status_definition_round_trips_position() {
        let mut def = status("open", "Open");
        def.position = 1500.0;
        let value = serde_json::to_value(&def).unwrap();
        assert_eq!(value.get("position"), Some(&serde_json::json!(1500.0)));
        let parsed: StatusDefinition = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.position, 1500.0);
    }

    #[test]
    fn status_definition_deserializes_legacy_payload_with_default_position() {
        // Older payloads (pre-position) had no `position`; deserialize to `0.0`.
        let legacy = serde_json::json!({
            "key": "open",
            "label": "Open",
            "color": "#abcdef",
            "unblocks_parents": false,
            "unblocks_dependents": false,
            "cascades_to_children": false,
        });
        let parsed: StatusDefinition = serde_json::from_value(legacy).unwrap();
        assert_eq!(parsed.position, 0.0);
    }

    #[test]
    fn upsert_project_request_round_trips_project_level_fields_only() {
        let mut req = UpsertProjectRequest::new(
            ProjectKey::try_new("eng").unwrap(),
            "Engineering".to_string(),
        );
        req.prompt_path = Some("/projects/eng/prompt.md".to_string());
        req.priority = 100.0;
        let value = serde_json::to_value(&req).unwrap();
        let mapping = value.as_object().expect("upsert request is a JSON object");
        let keys: std::collections::BTreeSet<_> = mapping.keys().cloned().collect();
        let expected: std::collections::BTreeSet<String> =
            ["key", "name", "prompt_path", "priority"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        assert_eq!(keys, expected);
        let parsed: UpsertProjectRequest = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.key, req.key);
        assert_eq!(parsed.name, req.name);
        assert_eq!(parsed.prompt_path, req.prompt_path);
        assert_eq!(parsed.priority, req.priority);
    }

    #[test]
    fn upsert_project_request_omits_prompt_path_when_none() {
        let req = UpsertProjectRequest::new(
            ProjectKey::try_new("eng").unwrap(),
            "Engineering".to_string(),
        );
        let value = serde_json::to_string(&req).unwrap();
        assert!(
            !value.contains("prompt_path"),
            "prompt_path should be skipped when None; got {value}"
        );
    }
}
