//! Shared [`Principal`] type for attribution fields (assignees, review
//! authors, merge-policy `any_of`).
//!
//! Introduced in Phase 1 of the actor-system overhaul
//! (`/designs/actor-system-overhaul.md`, §4.1). Later phases migrate
//! `Issue.assignee`, `Review.author`, and the existing
//! `crate::api::v1::repositories::Principal` to use this shared type.
//!
//! `Principal` is the durable subset of [`crate::actor_ref::ActorId`] —
//! `User`, `Agent`, `External`. It deliberately omits `Adhoc` (sessions
//! are transient identities) and `Legacy` (read-only catch-all).

use crate::api::v1::agents::AgentName;
use crate::api::v1::users::Username;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Validation failure for [`ExternalSystem::try_new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalSystemError {
    Empty,
    ContainsWhitespace,
    ContainsSlash,
}

impl fmt::Display for ExternalSystemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExternalSystemError::Empty => f.write_str("external system must not be empty"),
            ExternalSystemError::ContainsWhitespace => {
                f.write_str("external system must not contain whitespace")
            }
            ExternalSystemError::ContainsSlash => {
                f.write_str("external system must not contain '/'")
            }
        }
    }
}

impl std::error::Error for ExternalSystemError {}

/// A validated identifier for an external identity provider
/// (e.g. `github`).
///
/// Open string by design (clarification C4 in
/// `/designs/actor-system-overhaul.md`): we do not enumerate a closed
/// set. Known systems get UI affordances (icon, label), but any
/// well-formed identifier is accepted on the wire so new integrations
/// don't require an enum change.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, type = "string"))]
#[serde(transparent)]
#[non_exhaustive]
pub struct ExternalSystem(String);

impl ExternalSystem {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Validating constructor: rejects empty strings, whitespace, and
    /// `/`.
    pub fn try_new(value: impl Into<String>) -> Result<Self, ExternalSystemError> {
        let value = value.into();
        if value.is_empty() {
            return Err(ExternalSystemError::Empty);
        }
        if value.chars().any(char::is_whitespace) {
            return Err(ExternalSystemError::ContainsWhitespace);
        }
        if value.contains('/') {
            return Err(ExternalSystemError::ContainsSlash);
        }
        Ok(Self(value))
    }
}

impl fmt::Display for ExternalSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for ExternalSystem {
    type Err = ExternalSystemError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_new(s)
    }
}

impl AsRef<str> for ExternalSystem {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// A typed, validated party that owns or performed an action — the
/// shared attribution type for issues, reviews, and merge-policy lists.
///
/// `Principal` is `ActorId` minus the two transient variants
/// (`Adhoc`, `Legacy`): assigning durable work to an ad-hoc session
/// would dangle once the session ends, and `Legacy` exists only as a
/// read-only deserialization fallback.
///
/// **Wire form** is internally-tagged JSON per design §3.3:
///
/// ```jsonc
/// { "kind": "user",     "name":     "alice"   }
/// { "kind": "agent",    "name":     "swe"     }
/// { "kind": "external", "system":   "github", "username": "jayantk" }
/// ```
///
/// We use struct variants (`User { name }`, `Agent { name }`) rather
/// than the more concise tuple form (`User(Username)`) so the
/// internally-tagged wire format falls out of standard serde
/// derivation — `serde(tag = "kind")` is incompatible with tuple
/// variants. ts-rs derives the matching TypeScript shape automatically.
///
/// **Path form** (canonical, used in URLs, CLI args, and indexed DB
/// columns): `users/<x>` / `agents/<x>` / `external/<system>/<username>`.
///
/// **TS export rename:** this type is exported as `ActorPrincipal` in
/// Phase 1 to avoid a TS-file collision with the existing
/// [`crate::api::v1::repositories::Principal`] (which currently owns
/// `Principal.ts`). Phase 5/6 of the design unifies the two types and
/// the rename can be removed then.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, rename = "ActorPrincipal"))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Principal {
    User {
        name: Username,
    },
    Agent {
        name: AgentName,
    },
    External {
        system: ExternalSystem,
        username: String,
    },
}

impl Principal {
    /// Construct a [`Principal::User`].
    pub fn user(name: Username) -> Self {
        Principal::User { name }
    }

    /// Construct a [`Principal::Agent`].
    pub fn agent(name: AgentName) -> Self {
        Principal::Agent { name }
    }

    /// Construct a [`Principal::External`].
    pub fn external(system: ExternalSystem, username: impl Into<String>) -> Self {
        Principal::External {
            system,
            username: username.into(),
        }
    }

    /// Render this principal as its canonical path form (`users/<x>`,
    /// `agents/<x>`, `external/<system>/<username>`).
    pub fn to_path(&self) -> String {
        match self {
            Principal::User { name } => format!("users/{}", name.as_str()),
            Principal::Agent { name } => format!("agents/{}", name.as_str()),
            Principal::External { system, username } => {
                format!("external/{}/{}", system.as_str(), username)
            }
        }
    }

    /// Apply the Phase 4a/4b backfill heuristic to a raw legacy
    /// `Issue.assignee` string, producing a [`Principal`]:
    ///
    /// 1. If the input parses via [`Principal::from_str`] (canonical path
    ///    form `users/<x>` / `agents/<x>` / `external/<system>/<x>`), use
    ///    that.
    /// 2. Else if the input is a syntactically valid [`Username`], return
    ///    `Principal::User { name }`.
    /// 3. Else return `None`.
    ///
    /// This is the Rust-side mirror of the SQL `CASE` expression used by
    /// the v1 → v2 row migration; both sites need the same rule, so it
    /// lives here next to [`Principal::from_str`] / [`Principal::to_path`].
    pub fn parse_legacy_assignee(value: &str) -> Option<Self> {
        if let Ok(p) = Self::from_str(value) {
            return Some(p);
        }
        if let Ok(name) = Username::try_new(value) {
            return Some(Principal::User { name });
        }
        None
    }
}

impl fmt::Display for Principal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_path())
    }
}

/// Parse-failure for [`Principal::from_str`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrincipalParseError {
    Empty,
    UnknownKind(String),
    MissingSegment(&'static str),
    InvalidUsername(crate::api::v1::users::UsernameError),
    InvalidAgentName(crate::api::v1::agents::AgentNameError),
    InvalidExternalSystem(ExternalSystemError),
    EmptyExternalUsername,
}

impl fmt::Display for PrincipalParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrincipalParseError::Empty => f.write_str("principal path must not be empty"),
            PrincipalParseError::UnknownKind(k) => write!(
                f,
                "unknown principal kind '{k}'; expected one of users/, agents/, external/"
            ),
            PrincipalParseError::MissingSegment(what) => {
                write!(f, "principal path is missing segment '{what}'")
            }
            PrincipalParseError::InvalidUsername(e) => write!(f, "invalid username: {e}"),
            PrincipalParseError::InvalidAgentName(e) => write!(f, "invalid agent name: {e}"),
            PrincipalParseError::InvalidExternalSystem(e) => {
                write!(f, "invalid external system: {e}")
            }
            PrincipalParseError::EmptyExternalUsername => {
                f.write_str("external principal username must not be empty")
            }
        }
    }
}

impl std::error::Error for PrincipalParseError {}

impl FromStr for Principal {
    type Err = PrincipalParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(PrincipalParseError::Empty);
        }
        let (kind, rest) = s
            .split_once('/')
            .ok_or_else(|| PrincipalParseError::UnknownKind(s.to_string()))?;
        match kind {
            "users" => {
                let name = Username::try_new(rest).map_err(PrincipalParseError::InvalidUsername)?;
                Ok(Principal::User { name })
            }
            "agents" => {
                let name =
                    AgentName::try_new(rest).map_err(PrincipalParseError::InvalidAgentName)?;
                Ok(Principal::Agent { name })
            }
            "external" => {
                let (system, username) = rest
                    .split_once('/')
                    .ok_or(PrincipalParseError::MissingSegment("username"))?;
                let system = ExternalSystem::try_new(system)
                    .map_err(PrincipalParseError::InvalidExternalSystem)?;
                if username.is_empty() {
                    return Err(PrincipalParseError::EmptyExternalUsername);
                }
                Ok(Principal::External {
                    system,
                    username: username.to_string(),
                })
            }
            other => Err(PrincipalParseError::UnknownKind(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn alice() -> Principal {
        Principal::user(Username::try_new("alice").unwrap())
    }
    fn swe() -> Principal {
        Principal::agent(AgentName::try_new("swe").unwrap())
    }
    fn gh_jayantk() -> Principal {
        Principal::external(ExternalSystem::try_new("github").unwrap(), "jayantk")
    }

    // --- ExternalSystem --------------------------------------------------

    #[test]
    fn external_system_accepts_well_formed() {
        let s = ExternalSystem::try_new("github").unwrap();
        assert_eq!(s.as_str(), "github");
    }

    #[test]
    fn external_system_rejects_empty() {
        assert_eq!(ExternalSystem::try_new(""), Err(ExternalSystemError::Empty));
    }

    #[test]
    fn external_system_rejects_whitespace() {
        assert_eq!(
            ExternalSystem::try_new("git hub"),
            Err(ExternalSystemError::ContainsWhitespace)
        );
    }

    #[test]
    fn external_system_rejects_slash() {
        assert_eq!(
            ExternalSystem::try_new("github/com"),
            Err(ExternalSystemError::ContainsSlash)
        );
    }

    // --- Principal serde round-trip --------------------------------------

    #[test]
    fn principal_user_serde_round_trip() {
        let p = alice();
        let value = serde_json::to_value(&p).unwrap();
        assert_eq!(value, json!({"kind": "user", "name": "alice"}));
        let back: Principal = serde_json::from_value(value).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn principal_agent_serde_round_trip() {
        let p = swe();
        let value = serde_json::to_value(&p).unwrap();
        assert_eq!(value, json!({"kind": "agent", "name": "swe"}));
        let back: Principal = serde_json::from_value(value).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn principal_external_serde_round_trip() {
        let p = gh_jayantk();
        let value = serde_json::to_value(&p).unwrap();
        assert_eq!(
            value,
            json!({"kind": "external", "system": "github", "username": "jayantk"})
        );
        let back: Principal = serde_json::from_value(value).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn principal_deserialize_unknown_kind_errors() {
        let err = serde_json::from_value::<Principal>(json!({"kind": "robot", "name": "r2"}))
            .unwrap_err();
        assert!(
            err.to_string().contains("robot"),
            "error should mention unknown kind: {err}"
        );
    }

    #[test]
    fn principal_deserialize_missing_field_errors() {
        let err = serde_json::from_value::<Principal>(json!({"kind": "user"})).unwrap_err();
        assert!(err.to_string().contains("name"));
    }

    // --- Principal Display / FromStr -------------------------------------

    #[test]
    fn principal_display_user() {
        assert_eq!(alice().to_string(), "users/alice");
    }

    #[test]
    fn principal_display_agent() {
        assert_eq!(swe().to_string(), "agents/swe");
    }

    #[test]
    fn principal_display_external() {
        assert_eq!(gh_jayantk().to_string(), "external/github/jayantk");
    }

    #[test]
    fn principal_from_str_user() {
        let p: Principal = "users/alice".parse().unwrap();
        assert_eq!(p, alice());
    }

    #[test]
    fn principal_from_str_agent() {
        let p: Principal = "agents/swe".parse().unwrap();
        assert_eq!(p, swe());
    }

    #[test]
    fn principal_from_str_external() {
        let p: Principal = "external/github/jayantk".parse().unwrap();
        assert_eq!(p, gh_jayantk());
    }

    #[test]
    fn principal_from_str_round_trips_display() {
        for p in [alice(), swe(), gh_jayantk()] {
            let s = p.to_string();
            let back: Principal = s.parse().unwrap();
            assert_eq!(back, p);
        }
    }

    #[test]
    fn principal_from_str_rejects_empty() {
        assert_eq!("".parse::<Principal>(), Err(PrincipalParseError::Empty));
    }

    #[test]
    fn principal_from_str_rejects_unknown_kind() {
        let err = "robots/r2".parse::<Principal>().unwrap_err();
        assert!(matches!(err, PrincipalParseError::UnknownKind(ref k) if k == "robots"));
    }

    #[test]
    fn principal_from_str_rejects_external_missing_username() {
        let err = "external/github".parse::<Principal>().unwrap_err();
        assert_eq!(err, PrincipalParseError::MissingSegment("username"));
    }

    #[test]
    fn principal_from_str_rejects_external_empty_username() {
        let err = "external/github/".parse::<Principal>().unwrap_err();
        assert_eq!(err, PrincipalParseError::EmptyExternalUsername);
    }

    #[test]
    fn principal_from_str_rejects_invalid_username() {
        let err = "users/alice bob".parse::<Principal>().unwrap_err();
        assert!(matches!(err, PrincipalParseError::InvalidUsername(_)));
    }

    #[test]
    fn principal_from_str_rejects_no_separator() {
        let err = "alice".parse::<Principal>().unwrap_err();
        assert!(matches!(err, PrincipalParseError::UnknownKind(ref k) if k == "alice"));
    }

    // --- parse_legacy_assignee (Phase 4a/4b backfill heuristic) ----------

    #[test]
    fn parse_legacy_assignee_accepts_canonical_path() {
        assert_eq!(Principal::parse_legacy_assignee("users/alice"), Some(alice()));
        assert_eq!(Principal::parse_legacy_assignee("agents/swe"), Some(swe()));
        assert_eq!(
            Principal::parse_legacy_assignee("external/github/jayantk"),
            Some(gh_jayantk())
        );
    }

    #[test]
    fn parse_legacy_assignee_falls_back_to_bare_username_as_user() {
        // Pre-typed v1 rows often carry just "alice" with no prefix; the
        // heuristic wraps those as `Principal::User`.
        assert_eq!(Principal::parse_legacy_assignee("alice"), Some(alice()));
    }

    #[test]
    fn parse_legacy_assignee_returns_none_for_invalid_input() {
        assert_eq!(Principal::parse_legacy_assignee(""), None);
        assert_eq!(Principal::parse_legacy_assignee("alice bob"), None);
    }
}
