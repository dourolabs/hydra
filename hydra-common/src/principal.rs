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
use std::collections::HashSet;
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
/// **Wire form** is externally-tagged JSON (ts-rs / serde default):
///
/// ```jsonc
/// { "User":     { "name": "alice"   } }
/// { "Agent":    { "name": "swe"     } }
/// { "External": { "system": "github", "username": "jayantk" } }
/// ```
///
/// We keep struct variants (`User { name }`, `Agent { name }`) rather
/// than the more concise tuple form (`User(Username)`); it's a style
/// choice that produces a TS shape (`{ User: { name: Username } }`)
/// close to the original internally-tagged form, keeping consumer
/// migration mechanical.
///
/// **Path form** (canonical, used in URLs, CLI args, and indexed DB
/// columns): `users/<x>` / `agents/<x>` / `external/<system>/<username>`.
///
/// Phase 5a of `/designs/actor-system-overhaul.md` unifies this with the
/// merge-policy principal: `crate::api::v1::repositories::Principal`
/// (the old bare-string wire enum) is gone, replaced by an `AssigneeRef`
/// wrapper that stores `Principal` for its static case. The Phase 1
/// `rename = "ActorPrincipal"` workaround is dropped here so the shared
/// type takes back ownership of `Principal.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
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
    /// Note: every bare string is classified as `Principal::User`. Migration
    /// callers that need to disambiguate users from agents (historically
    /// conflated in `Issue.assignee` / `Review.author`) should use
    /// [`Principal::parse_legacy_assignee_with_agents`] instead.
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

    /// Variant of [`Principal::parse_legacy_assignee`] that consults a
    /// known-agent-names set to disambiguate bare strings.
    ///
    /// Historically `Issue.assignee` and `Review.author` conflated user
    /// and agent names in the same bare-string column. The Phase 4a
    /// migration first classified every bare string as
    /// `Principal::User`, lifting agents like `swe` / `reviewer` /
    /// `merger` to the wrong kind. This variant fixes that by
    /// case-sensitively matching the input against `agent_names`
    /// before falling back to `Principal::User`.
    ///
    /// Classification order:
    /// 1. Canonical `users/<x>` / `agents/<x>` / `external/<sys>/<x>`
    ///    via [`Principal::from_str`].
    /// 2. Bare `<x>` where `agent_names` contains `<x>` and `<x>`
    ///    validates as an [`AgentName`] → `Principal::Agent`.
    /// 3. Bare `<x>` that validates as a [`Username`] → `Principal::User`.
    /// 4. `None`.
    ///
    /// `agent_names` should be the full set of agent names from the
    /// live `agents` table at migration time, including deleted
    /// entries — once a name was registered as an agent, legacy
    /// attribution strings for that name refer to the agent.
    pub fn parse_legacy_assignee_with_agents(
        value: &str,
        agent_names: &HashSet<String>,
    ) -> Option<Self> {
        if let Ok(p) = Self::from_str(value) {
            return Some(p);
        }
        if agent_names.contains(value) {
            if let Ok(name) = AgentName::try_new(value) {
                return Some(Principal::Agent { name });
            }
        }
        if let Ok(name) = Username::try_new(value) {
            return Some(Principal::User { name });
        }
        None
    }
}

/// Kind-aware, case-insensitive equality on [`Principal`].
///
/// `User` matches `User` by case-insensitive name; `Agent` matches `Agent`
/// by case-insensitive name; `External` matches `External` when both
/// `system` and `username` are case-insensitive equal. Mismatched kinds
/// never compare equal — a [`Principal::User`] named `swe` does not match a
/// [`Principal::Agent`] named `swe`.
///
/// Phase 6 of `/designs/actor-system-overhaul.md` uses this helper in
/// every merge-authorisation matching site (mergers list, reviewer-group
/// quorum, author exclusion) so the same principal-equality rule is
/// applied uniformly.
pub fn principal_eq(a: &Principal, b: &Principal) -> bool {
    match (a, b) {
        (Principal::User { name: n1 }, Principal::User { name: n2 }) => {
            n1.as_str().eq_ignore_ascii_case(n2.as_str())
        }
        (Principal::Agent { name: n1 }, Principal::Agent { name: n2 }) => {
            n1.as_str().eq_ignore_ascii_case(n2.as_str())
        }
        (
            Principal::External {
                system: s1,
                username: u1,
            },
            Principal::External {
                system: s2,
                username: u2,
            },
        ) => s1.as_str().eq_ignore_ascii_case(s2.as_str()) && u1.eq_ignore_ascii_case(u2),
        _ => false,
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
    fn principal_deserialize_unknown_variant_errors() {
        let err =
            serde_json::from_value::<Principal>(json!({"Robot": {"name": "r2"}})).unwrap_err();
        assert!(
            err.to_string().contains("Robot"),
            "error should mention unknown variant: {err}"
        );
    }

    #[test]
    fn principal_deserialize_missing_field_errors() {
        let err = serde_json::from_value::<Principal>(json!({"User": {}})).unwrap_err();
        assert!(err.to_string().contains("name"));
    }

    // --- Principal Display / FromStr -------------------------------------

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
        assert_eq!(
            Principal::parse_legacy_assignee("users/alice"),
            Some(alice())
        );
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

    // --- parse_legacy_assignee_with_agents (Phase 4a fix) ----------------

    fn agent_set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_legacy_assignee_with_agents_canonical_path_unchanged() {
        let agents = agent_set(&["swe", "reviewer"]);
        assert_eq!(
            Principal::parse_legacy_assignee_with_agents("users/alice", &agents),
            Some(alice())
        );
        assert_eq!(
            Principal::parse_legacy_assignee_with_agents("agents/swe", &agents),
            Some(swe())
        );
        assert_eq!(
            Principal::parse_legacy_assignee_with_agents("external/github/jayantk", &agents),
            Some(gh_jayantk())
        );
    }

    #[test]
    fn parse_legacy_assignee_with_agents_bare_name_in_set_classifies_as_agent() {
        // Pre-Phase-4a-fix would have lifted bare "swe" to Principal::User.
        // With the agent-name set, we correctly classify it as an agent.
        let agents = agent_set(&["swe", "reviewer", "merger", "pm"]);
        assert_eq!(
            Principal::parse_legacy_assignee_with_agents("swe", &agents),
            Some(swe())
        );
        assert_eq!(
            Principal::parse_legacy_assignee_with_agents("reviewer", &agents),
            Some(Principal::Agent {
                name: AgentName::try_new("reviewer").unwrap()
            })
        );
    }

    #[test]
    fn parse_legacy_assignee_with_agents_bare_name_not_in_set_classifies_as_user() {
        let agents = agent_set(&["swe", "reviewer"]);
        assert_eq!(
            Principal::parse_legacy_assignee_with_agents("alice", &agents),
            Some(alice())
        );
    }

    #[test]
    fn parse_legacy_assignee_with_agents_empty_agent_set_matches_old_behavior() {
        // With no agents registered, every bare name is a user — same as
        // `parse_legacy_assignee` before the fix.
        let agents = HashSet::new();
        assert_eq!(
            Principal::parse_legacy_assignee_with_agents("alice", &agents),
            Some(alice())
        );
        assert_eq!(
            Principal::parse_legacy_assignee_with_agents("swe", &agents),
            Some(Principal::User {
                name: Username::try_new("swe").unwrap()
            })
        );
    }

    #[test]
    fn parse_legacy_assignee_with_agents_case_sensitive_match() {
        // Case-sensitive: "SWE" (uppercase) does NOT match the registered
        // lowercase "swe" agent name, so it falls back to Principal::User.
        // The agents table is case-sensitive (TEXT PRIMARY KEY in SQLite,
        // unqualified TEXT in postgres), so we mirror that here.
        let agents = agent_set(&["swe"]);
        assert_eq!(
            Principal::parse_legacy_assignee_with_agents("SWE", &agents),
            Some(Principal::User {
                name: Username::try_new("SWE").unwrap()
            })
        );
    }

    #[test]
    fn parse_legacy_assignee_with_agents_returns_none_for_invalid_input() {
        let agents = agent_set(&["swe"]);
        assert_eq!(
            Principal::parse_legacy_assignee_with_agents("", &agents),
            None
        );
        // Whitespace-bearing strings fail Username validation and the
        // agent-name set is consulted with the raw value (which also
        // wouldn't validate as an AgentName).
        assert_eq!(
            Principal::parse_legacy_assignee_with_agents("alice bob", &agents),
            None
        );
    }

    #[test]
    fn parse_legacy_assignee_with_agents_known_agent_path_form_still_works() {
        // Even if "swe" is in the agent set, `agents/swe` should be
        // parsed via `Principal::from_str` first and not via the bare
        // path. The end result is the same Principal::Agent.
        let agents = agent_set(&["swe"]);
        assert_eq!(
            Principal::parse_legacy_assignee_with_agents("agents/swe", &agents),
            Some(swe())
        );
    }

    // --- principal_eq (Phase 6 kind-aware case-insensitive equality) -----

    #[test]
    fn principal_eq_user_case_insensitive() {
        let upper = Principal::user(Username::try_new("ALICE").unwrap());
        assert!(principal_eq(&alice(), &upper));
    }

    #[test]
    fn principal_eq_agent_case_insensitive() {
        let swe_lower = Principal::agent(AgentName::try_new("swe").unwrap());
        let swe_mixed = Principal::agent(AgentName::try_new("sWE").unwrap());
        let swe_upper = Principal::agent(AgentName::try_new("Swe").unwrap());
        assert!(principal_eq(&swe_lower, &swe_mixed));
        assert!(principal_eq(&swe_lower, &swe_upper));
        assert!(principal_eq(&swe_mixed, &swe_upper));
    }

    #[test]
    fn principal_eq_user_does_not_match_agent_with_same_name() {
        let swe_user = Principal::user(Username::try_new("swe").unwrap());
        let swe_agent = Principal::agent(AgentName::try_new("swe").unwrap());
        assert!(!principal_eq(&swe_user, &swe_agent));
        assert!(!principal_eq(&swe_agent, &swe_user));
    }

    #[test]
    fn principal_eq_external_does_not_match_user_with_same_name() {
        let user_x = Principal::user(Username::try_new("x").unwrap());
        let ext_x = Principal::external(ExternalSystem::try_new("github").unwrap(), "x");
        assert!(!principal_eq(&user_x, &ext_x));
        assert!(!principal_eq(&ext_x, &user_x));
    }

    #[test]
    fn principal_eq_external_matches_same_system_and_username_ci() {
        let lower = Principal::external(ExternalSystem::try_new("github").unwrap(), "jayantk");
        let upper = Principal::external(ExternalSystem::try_new("GitHub").unwrap(), "JayantK");
        assert!(principal_eq(&lower, &upper));
    }

    #[test]
    fn principal_eq_external_differs_by_system() {
        let gh = Principal::external(ExternalSystem::try_new("github").unwrap(), "alice");
        let gl = Principal::external(ExternalSystem::try_new("gitlab").unwrap(), "alice");
        assert!(!principal_eq(&gh, &gl));
    }

    #[test]
    fn principal_eq_external_differs_by_username() {
        let alice = Principal::external(ExternalSystem::try_new("github").unwrap(), "alice");
        let bob = Principal::external(ExternalSystem::try_new("github").unwrap(), "bob");
        assert!(!principal_eq(&alice, &bob));
    }
}
