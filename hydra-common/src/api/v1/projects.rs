//! Per-project status configuration types.
//!
//! See `/designs/per-project-issue-statuses.md` §4 "Core shapes" for the full
//! design. PR 1/6 (this PR) introduces the wire types and a `DefaultProject`
//! fallback const; no store, routes, or consumers are wired up yet.

use crate::document_path::DocumentPath;
use crate::principal::Principal;
use crate::{Rgb, api::v1::users::Username, ids::ProjectId};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

/// Maximum length for project / status / icon keys. Keeps wire strings
/// bounded and well below any reasonable index limit.
pub const MAX_KEY_LENGTH: usize = 64;

/// Validation failure for newtyped string keys ([`ProjectKey`],
/// [`StatusKey`], [`IconKey`]).
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
define_key_newtype!(
    IconKey,
    "Frontend icon identifier (resolved against the theme's icon set)."
);

/// Automation rule fired when an issue's status transitions into a status
/// declaring `on_enter`. See `/designs/per-project-issue-statuses.md` §4
/// "Spawn dispatch and on_enter automation".
///
/// PR 1/6 carries the wire type only; the automation that consumes it
/// lands in PR 4.
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
    pub icon: IconKey,
    pub color: Rgb,
    pub unblocks_parents: bool,
    pub unblocks_dependents: bool,
    pub cascades_to_children: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_enter: Option<StatusOnEnter>,
}

impl StatusDefinition {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        key: StatusKey,
        label: String,
        icon: IconKey,
        color: Rgb,
        unblocks_parents: bool,
        unblocks_dependents: bool,
        cascades_to_children: bool,
        on_enter: Option<StatusOnEnter>,
    ) -> Self {
        Self {
            key,
            label,
            icon,
            color,
            unblocks_parents,
            unblocks_dependents,
            cascades_to_children,
            on_enter,
        }
    }
}

/// A project owns an ordered list of [`StatusDefinition`]s plus an explicit
/// [`Self::default_status_key`] for new issues. See
/// `/designs/per-project-issue-statuses.md` §4.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct Project {
    pub key: ProjectKey,
    pub name: String,
    pub statuses: Vec<StatusDefinition>,
    pub default_status_key: StatusKey,
    pub creator: Username,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
}

/// Validation failure for [`Project::validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectValidationError {
    DuplicateStatusKey(StatusKey),
    DefaultStatusKeyMissing(StatusKey),
    NoStatuses,
}

impl fmt::Display for ProjectValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectValidationError::DuplicateStatusKey(key) => {
                write!(f, "duplicate status key '{key}' in project")
            }
            ProjectValidationError::DefaultStatusKeyMissing(key) => {
                write!(
                    f,
                    "default_status_key '{key}' does not reference any status in the project"
                )
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
        default_status_key: StatusKey,
        creator: Username,
        deleted: bool,
    ) -> Self {
        Self {
            key,
            name,
            statuses,
            default_status_key,
            creator,
            deleted,
        }
    }

    /// Check structural invariants:
    /// - statuses is non-empty
    /// - all status keys are unique within the project
    /// - `default_status_key` references an entry in `statuses`
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
        if !seen.contains(&self.default_status_key) {
            return Err(ProjectValidationError::DefaultStatusKeyMissing(
                self.default_status_key.clone(),
            ));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn status(key: &str, label: &str) -> StatusDefinition {
        StatusDefinition::new(
            StatusKey::try_new(key).unwrap(),
            label.to_string(),
            IconKey::try_new("circle").unwrap(),
            "#abcdef".parse().unwrap(),
            false,
            false,
            false,
            None,
        )
    }

    fn project(default_key: &str, statuses: Vec<StatusDefinition>) -> Project {
        Project::new(
            ProjectKey::try_new("eng").unwrap(),
            "Engineering".to_string(),
            statuses,
            StatusKey::try_new(default_key).unwrap(),
            Username::try_new("jayantk").unwrap(),
            false,
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
        let proj = project("in-progress", vec![def]);

        let json = serde_json::to_string(&proj).unwrap();
        let parsed: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, proj);
        // The exact wire field name should be `default_status_key` so older
        // clients see a stable contract.
        assert!(json.contains("\"default_status_key\""));
    }

    #[test]
    fn status_definition_omits_on_enter_when_none() {
        let def = status("open", "Open");
        let json = serde_json::to_string(&def).unwrap();
        assert!(!json.contains("on_enter"));
    }

    #[test]
    fn project_validate_accepts_well_formed() {
        let proj = project(
            "open",
            vec![status("open", "Open"), status("closed", "Closed")],
        );
        proj.validate().unwrap();
    }

    #[test]
    fn project_validate_rejects_duplicate_status_keys() {
        let proj = project(
            "open",
            vec![status("open", "Open"), status("open", "Open Again")],
        );
        let err = proj.validate().unwrap_err();
        assert!(matches!(err, ProjectValidationError::DuplicateStatusKey(_)));
    }

    #[test]
    fn project_validate_rejects_missing_default_status_key() {
        let proj = project("missing", vec![status("open", "Open")]);
        let err = proj.validate().unwrap_err();
        assert!(matches!(
            err,
            ProjectValidationError::DefaultStatusKeyMissing(_)
        ));
    }

    #[test]
    fn project_validate_rejects_empty_status_list() {
        let proj = project("open", vec![]);
        let err = proj.validate().unwrap_err();
        assert!(matches!(err, ProjectValidationError::NoStatuses));
    }

    #[test]
    fn find_status_returns_matching_definition() {
        let proj = project(
            "open",
            vec![status("open", "Open"), status("closed", "Closed")],
        );
        let key = StatusKey::try_new("closed").unwrap();
        let found = proj.find_status(&key).unwrap();
        assert_eq!(found.label, "Closed");
    }

    #[test]
    fn find_status_returns_none_for_unknown_key() {
        let proj = project("open", vec![status("open", "Open")]);
        let key = StatusKey::try_new("nope").unwrap();
        assert!(proj.find_status(&key).is_none());
    }
}
