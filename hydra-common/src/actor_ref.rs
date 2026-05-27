use crate::api::v1::agents::AgentName;
use crate::api::v1::users::Username;
use crate::ids::{IssueId, SessionId};
use crate::principal::ExternalSystem;
use crate::whoami::ActorIdentity;
use serde::de::{Error as DeError, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// Typed identifier for the principal that performed an operation.
///
/// Phase 1 of the actor-system overhaul
/// (`/designs/actor-system-overhaul.md`, §3.1) introduces five new
/// variants (`User`, `Agent`, `Adhoc`, `External`, `Legacy`) alongside
/// the existing four (`Username`, `Session`, `Issue`, `Service`). Call
/// sites flip over to the new variants incrementally in Phases 2–6;
/// the old variants are removed in a release-gated cleanup PR (§11
/// row 7) after a soak window with zero `Legacy` deserializations.
///
/// Variants:
/// - **`User(Username)`** — replaces `Username` once call sites migrate.
/// - **`Agent(AgentName)`** — first-class named agent (e.g. `pm`, `swe`).
/// - **`Adhoc(SessionId)`** — a session created outside the agent
///   system. Canonical path is `adhoc/<session-id>` (clarification C2
///   in the design — *not* `sessions/`).
/// - **`External { system, username }`** — an identity that lives in an
///   external system (e.g. GitHub) and has no corresponding Hydra user.
/// - **`Legacy(String)`** — read-only deserialization catch-all for
///   pre-migration `actor_ref` blobs. New writes must not produce this
///   variant; it round-trips losslessly as a raw string so unmigrated
///   rows aren't corrupted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
// ActorId has a hand-rolled serde impl (below) that emits the
// externally-tagged form for every variant except `Legacy(String)`,
// which round-trips as a bare JSON string. ts-rs derives the
// externally-tagged union for us; we override the TS body only to
// append the `| string` fallback that models the bare-string
// `Legacy` payload (ts-rs has no way to express that automatically).
// The referenced newtypes (`Username`, `AgentName`, `SessionId`,
// `IssueId`, `ExternalSystem`) are all `type X = string;` aliases
// today, so plain `string` is the accurate TS shape. The Rust serde
// tests in this file pin the wire form on the Rust side; the matching
// TS-side fixture test lives in
// `hydra-web/packages/web/src/utils/actors.test.ts`.
#[cfg_attr(
    feature = "ts",
    ts(
        export,
        type = "\
| { Username: string } \
| { Session: string } \
| { Issue: string } \
| { Service: string } \
| { User: { name: string } } \
| { Agent: { name: string } } \
| { Adhoc: { session_id: string } } \
| { External: { system: string; username: string } } \
| string"
    )
)]
pub enum ActorId {
    // Existing variants — kept until the release-gated cleanup PR.
    Username(Username),
    Session(SessionId),
    Issue(IssueId),
    Service(String),
    // Phase-1 additions — see module-level docs above.
    User(Username),
    Agent(AgentName),
    Adhoc(SessionId),
    External {
        system: ExternalSystem,
        username: String,
    },
    /// Read-only fallback for pre-migration data. Never produced by new
    /// writes — any `ActorId::new_*`-style helper added in later phases
    /// must `debug_assert!(!matches!(_, ActorId::Legacy(_)))`.
    Legacy(String),
}

/// Display name for an [`ActorId`] used by [`ActorRef::display_name`].
///
/// Existing variants keep the same bare/prefixed format they've always
/// rendered with; new variants use their canonical path form.
fn actor_id_display_name(actor_id: &ActorId) -> String {
    match actor_id {
        ActorId::Username(username) => username.to_string(),
        ActorId::Session(session_id) => session_id.to_string(),
        ActorId::Issue(issue_id) => issue_id.to_string(),
        ActorId::Service(name) => format!("svc-{name}"),
        ActorId::User(u) => u.as_str().to_string(),
        ActorId::Agent(a) => a.as_str().to_string(),
        ActorId::Adhoc(s) => s.to_string(),
        ActorId::External { system, username } => {
            format!("external/{}/{}", system.as_str(), username)
        }
        ActorId::Legacy(raw) => raw.clone(),
    }
}

impl fmt::Display for ActorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Existing prefixed shorthand — unchanged.
            ActorId::Username(username) => write!(f, "u-{username}"),
            ActorId::Session(session_id) => write!(f, "w-{session_id}"),
            ActorId::Issue(issue_id) => write!(f, "a-{issue_id}"),
            ActorId::Service(name) => write!(f, "svc-{name}"),
            // New variants — canonical path form per design §3.3.
            ActorId::User(u) => write!(f, "users/{}", u.as_str()),
            ActorId::Agent(a) => write!(f, "agents/{}", a.as_str()),
            ActorId::Adhoc(s) => write!(f, "adhoc/{s}"),
            ActorId::External { system, username } => {
                write!(f, "external/{}/{}", system.as_str(), username)
            }
            ActorId::Legacy(raw) => f.write_str(raw),
        }
    }
}

/// A typed reference to who performed an operation.
///
/// Used in event payloads (`MutationPayload`) to attribute mutations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum ActorRef {
    Authenticated {
        actor_id: ActorId,
        /// Session that minted the authenticating token, if any.
        ///
        /// Phase 3a of `/designs/actor-system-overhaul.md` (§5.2) carries
        /// the originating session id end-to-end on session-spawned
        /// actors (`ActorId::Agent` / `ActorId::Adhoc`). User logins are
        /// not session-scoped, so `None` is valid for those.
        ///
        /// `#[serde(default)]` keeps existing version-history rows (which
        /// predate this field) deserializing as `session_id: None`
        /// (§5.4).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<SessionId>,
    },
    System {
        worker_name: String,
        on_behalf_of: Option<ActorId>,
    },
    Automation {
        automation_name: String,
        triggered_by: Option<Box<ActorRef>>,
    },
}

impl ActorRef {
    /// Human-readable display name for this actor reference.
    pub fn display_name(&self) -> String {
        match self {
            ActorRef::Authenticated { actor_id, .. } => actor_id_display_name(actor_id),
            ActorRef::System {
                worker_name,
                on_behalf_of,
            } => {
                if let Some(behalf) = on_behalf_of {
                    let behalf_name = actor_id_display_name(behalf);
                    format!("{worker_name} (on behalf of {behalf_name})")
                } else {
                    worker_name.clone()
                }
            }
            ActorRef::Automation {
                automation_name,
                triggered_by,
            } => {
                if let Some(trigger) = triggered_by {
                    format!(
                        "{automation_name} (triggered by {})",
                        trigger.display_name()
                    )
                } else {
                    automation_name.clone()
                }
            }
        }
    }

    /// Returns a test helper `ActorRef` for use in tests.
    pub fn test() -> ActorRef {
        ActorRef::System {
            worker_name: "test".into(),
            on_behalf_of: None,
        }
    }

    /// Resolve this actor reference to the underlying principal `ActorId`.
    ///
    /// `Authenticated` returns its own `actor_id`. `System` returns its
    /// `on_behalf_of` actor if set. `Automation` recursively resolves through
    /// its `triggered_by` chain so a chain like
    /// `Automation -> Automation -> Authenticated(u-alice)` resolves to
    /// `Username("alice")`.
    pub fn on_behalf_of(&self) -> Option<ActorId> {
        match self {
            ActorRef::Authenticated { actor_id, .. } => Some(actor_id.clone()),
            ActorRef::System { on_behalf_of, .. } => on_behalf_of.clone(),
            ActorRef::Automation { triggered_by, .. } => {
                triggered_by.as_ref().and_then(|t| t.on_behalf_of())
            }
        }
    }

    /// Return the `session_id` of the innermost `Authenticated` actor reached
    /// by walking the same `Authenticated | Automation.triggered_by` chain
    /// that [`on_behalf_of`] walks.
    ///
    /// Used by policy automations that need to recover the originating
    /// session for `ActorId::Agent` / `ActorId::Adhoc` writes — `on_behalf_of`
    /// returns the bare `ActorId` and discards the surrounding `session_id`
    /// field, so consumers that want both must call this helper alongside it.
    ///
    /// `System { on_behalf_of }` carries an `ActorId` rather than a nested
    /// `ActorRef`, so its branch returns `None` — there is no inner
    /// `Authenticated` to recurse into.
    pub fn originating_session_id(&self) -> Option<&SessionId> {
        match self {
            ActorRef::Authenticated { session_id, .. } => session_id.as_ref(),
            ActorRef::System { .. } => None,
            ActorRef::Automation { triggered_by, .. } => triggered_by
                .as_ref()
                .and_then(|t| t.originating_session_id()),
        }
    }
}

/// Parse a user-facing shorthand string into an `ActorId`.
///
/// Recognised forms, in priority order:
/// - Canonical Phase-1 path forms (design §3.3):
///   `users/<x>`, `agents/<x>`, `adhoc/<x>`, `external/<sys>/<x>`.
/// - Existing shorthand kept for back-compat with the pre-Phase-1
///   CLI / wire surface: `i-…` → `Issue`, `s-…` → `Session`,
///   `svc-…` → `Service`, anything else → `Username`.
///
/// **Note:** this `FromStr` deliberately does NOT round-trip with
/// [`Display`] for the existing variants — `Display` uses the canonical
/// prefixed format (`u-`, `a-`, `w-`) while this `FromStr` consumes the
/// shorthand input form. Use [`parse_actor_name`] to parse the
/// canonical prefixed-`Display` format.
impl FromStr for ActorId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err("actor ID must not be empty".to_string());
        }

        // Phase-1 canonical path forms (design §3.3).
        if let Some(name) = trimmed.strip_prefix("users/") {
            let u = Username::try_new(name)
                .map_err(|e| format!("invalid user path '{trimmed}': {e}"))?;
            return Ok(ActorId::User(u));
        }
        if let Some(name) = trimmed.strip_prefix("agents/") {
            let a = AgentName::try_new(name)
                .map_err(|e| format!("invalid agent path '{trimmed}': {e}"))?;
            return Ok(ActorId::Agent(a));
        }
        if let Some(sid_str) = trimmed.strip_prefix("adhoc/") {
            let sid = SessionId::from_str(sid_str)
                .map_err(|e| format!("invalid adhoc session id '{sid_str}': {e}"))?;
            return Ok(ActorId::Adhoc(sid));
        }
        if let Some(rest) = trimmed.strip_prefix("external/") {
            let (system, username) = rest.split_once('/').ok_or_else(|| {
                format!("external actor must be 'external/<system>/<username>': {trimmed}")
            })?;
            let sys = ExternalSystem::try_new(system)
                .map_err(|e| format!("invalid external system '{system}': {e}"))?;
            if username.is_empty() {
                return Err(format!(
                    "external actor username must not be empty: {trimmed}"
                ));
            }
            return Ok(ActorId::External {
                system: sys,
                username: username.to_string(),
            });
        }

        // Existing pre-Phase-1 prefixed shorthand — kept until call sites migrate.
        if trimmed.starts_with("i-") {
            let issue_id = IssueId::from_str(trimmed)
                .map_err(|e| format!("invalid issue ID '{trimmed}': {e}"))?;
            return Ok(ActorId::Issue(issue_id));
        }

        if trimmed.starts_with("s-") {
            let session_id = SessionId::from_str(trimmed)
                .map_err(|e| format!("invalid session ID '{trimmed}': {e}"))?;
            return Ok(ActorId::Session(session_id));
        }

        if let Some(service_name) = trimmed.strip_prefix("svc-") {
            if service_name.is_empty() {
                return Err("service name must not be empty".to_string());
            }
            return Ok(ActorId::Service(service_name.to_string()));
        }

        Ok(ActorId::Username(Username::from(trimmed)))
    }
}

impl TryFrom<ActorIdentity> for ActorId {
    type Error = String;

    fn try_from(identity: ActorIdentity) -> Result<Self, Self::Error> {
        #[allow(unreachable_patterns)]
        match identity {
            ActorIdentity::User { username } => Ok(ActorId::Username(username)),
            ActorIdentity::Session { session_id, .. } => Ok(ActorId::Session(session_id)),
            ActorIdentity::Issue { issue_id, .. } => Ok(ActorId::Issue(issue_id)),
            ActorIdentity::Service { service_name } => Ok(ActorId::Service(service_name)),
            _ => Err("unsupported actor identity type".to_string()),
        }
    }
}

/// Parse an actor name string (e.g. `u-alice`, `w-s-abcdef`, or
/// `agents/swe`) into an `ActorId`.
///
/// Returns `None` if the name does not match a recognized form or is
/// otherwise invalid. Both the pre-Phase-1 prefixed shorthand
/// (`u-`/`a-`/`w-`/`svc-`) AND the Phase-1 canonical path forms
/// (`users/<x>`, `agents/<x>`, `adhoc/<x>`, `external/<sys>/<x>`) are
/// accepted: the auth layer's `AuthToken::parse` calls this on the
/// token prefix, and Phase-2 lets `create_actor_for_job` mint tokens
/// whose prefix uses the new agent/adhoc display form (see
/// `/designs/actor-system-overhaul.md` §3.4).
pub fn parse_actor_name(name: &str) -> Option<ActorId> {
    if let Some(username) = name.strip_prefix("u-") {
        if username.is_empty() {
            return None;
        }
        return Some(ActorId::Username(Username::from(username)));
    }

    if let Some(rest) = name.strip_prefix("a-") {
        if rest.is_empty() {
            return None;
        }
        let issue_id = IssueId::from_str(rest).ok()?;
        return Some(ActorId::Issue(issue_id));
    }

    if let Some(task_id) = name.strip_prefix("w-") {
        if task_id.is_empty() {
            return None;
        }
        let task_id = SessionId::from_str(task_id).ok()?;
        return Some(ActorId::Session(task_id));
    }

    if let Some(service_name) = name.strip_prefix("svc-") {
        if service_name.is_empty() {
            return None;
        }
        return Some(ActorId::Service(service_name.to_string()));
    }

    // Phase-1 canonical path forms (design §3.3). Used by the auth
    // layer once `create_actor_for_job` mints `ActorId::Agent` /
    // `ActorId::Adhoc` tokens (Phase 2).
    if let Some(rest) = name.strip_prefix("users/") {
        let u = Username::try_new(rest).ok()?;
        return Some(ActorId::User(u));
    }

    if let Some(rest) = name.strip_prefix("agents/") {
        let a = AgentName::try_new(rest).ok()?;
        return Some(ActorId::Agent(a));
    }

    if let Some(rest) = name.strip_prefix("adhoc/") {
        let sid = SessionId::from_str(rest).ok()?;
        return Some(ActorId::Adhoc(sid));
    }

    if let Some(rest) = name.strip_prefix("external/") {
        let (system, username) = rest.split_once('/')?;
        if username.is_empty() {
            return None;
        }
        let sys = ExternalSystem::try_new(system).ok()?;
        return Some(ActorId::External {
            system: sys,
            username: username.to_string(),
        });
    }

    None
}

// -------------------------------------------------------------------------
// Serde — bespoke impls.
//
// Wire format (externally-tagged for every typed variant; `Legacy`
// round-trips as a bare string):
//   {"Username": "alice"}
//   {"Session":  "s-..."}
//   {"Issue":    "i-..."}
//   {"Service":  "bff"}
//   {"User":     {"name":       "alice"}}
//   {"Agent":    {"name":       "swe"}}
//   {"Adhoc":    {"session_id": "s-..."}}
//   {"External": {"system":     "github", "username": "jayantk"}}
//   "raw-legacy-blob"
//
// Any payload the deserializer can't interpret (unknown variant tag,
// free string, …) lands in `Legacy(...)` so pre-migration rows
// survive a load → save cycle untouched.
// -------------------------------------------------------------------------

impl Serialize for ActorId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            ActorId::Username(u) => {
                let mut m = serializer.serialize_map(Some(1))?;
                m.serialize_entry("Username", u.as_str())?;
                m.end()
            }
            ActorId::Session(s) => {
                let mut m = serializer.serialize_map(Some(1))?;
                m.serialize_entry("Session", &s.to_string())?;
                m.end()
            }
            ActorId::Issue(i) => {
                let mut m = serializer.serialize_map(Some(1))?;
                m.serialize_entry("Issue", &i.to_string())?;
                m.end()
            }
            ActorId::Service(name) => {
                let mut m = serializer.serialize_map(Some(1))?;
                m.serialize_entry("Service", name)?;
                m.end()
            }
            ActorId::User(u) => {
                let mut m = serializer.serialize_map(Some(1))?;
                m.serialize_entry("User", &serde_json::json!({ "name": u.as_str() }))?;
                m.end()
            }
            ActorId::Agent(a) => {
                let mut m = serializer.serialize_map(Some(1))?;
                m.serialize_entry("Agent", &serde_json::json!({ "name": a.as_str() }))?;
                m.end()
            }
            ActorId::Adhoc(s) => {
                let mut m = serializer.serialize_map(Some(1))?;
                m.serialize_entry("Adhoc", &serde_json::json!({ "session_id": s.to_string() }))?;
                m.end()
            }
            ActorId::External { system, username } => {
                let mut m = serializer.serialize_map(Some(1))?;
                m.serialize_entry(
                    "External",
                    &serde_json::json!({
                        "system": system.as_str(),
                        "username": username,
                    }),
                )?;
                m.end()
            }
            ActorId::Legacy(raw) => serializer.serialize_str(raw),
        }
    }
}

impl<'de> Deserialize<'de> for ActorId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ActorIdVisitor)
    }
}

struct ActorIdVisitor;

impl<'de> Visitor<'de> for ActorIdVisitor {
    type Value = ActorId;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an ActorId map (externally-tagged), or a string (read-only Legacy fallback)")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        tracing::warn!(
            target: "actor_id_legacy_decode",
            raw = %v,
            "deserialized ActorId::Legacy from bare string",
        );
        Ok(ActorId::Legacy(v.to_string()))
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        tracing::warn!(
            target: "actor_id_legacy_decode",
            raw = %v,
            "deserialized ActorId::Legacy from bare string",
        );
        Ok(ActorId::Legacy(v))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        // Collect every key/value entry up-front so we can route by the
        // single-entry tag (externally-tagged form) without committing
        // to a particular ordering.
        let mut entries: Vec<(String, serde_json::Value)> = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            let value: serde_json::Value = map.next_value()?;
            entries.push((key, value));
        }

        // Externally-tagged form: exactly one entry, key = variant name.
        if entries.len() == 1 {
            let (tag, value) = &entries[0];
            if let Some(parsed) = try_parse_external(tag, value).transpose() {
                return parsed.map_err(A::Error::custom);
            }
        }

        // Anything else: capture as Legacy(raw JSON).
        Ok(ActorId::Legacy(entries_to_legacy_string(&entries)))
    }
}

fn try_parse_external(tag: &str, value: &serde_json::Value) -> Result<Option<ActorId>, String> {
    let str_field = |obj: &serde_json::Value, name: &'static str| -> Result<String, String> {
        let v = obj
            .get(name)
            .ok_or_else(|| format!("missing field '{name}'"))?;
        v.as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| format!("field '{name}' must be a string"))
    };
    match tag {
        "Username" => {
            let s = value
                .as_str()
                .ok_or_else(|| "'Username' must be a string".to_string())?;
            Ok(Some(ActorId::Username(Username::from(s))))
        }
        "Session" => {
            let s = value
                .as_str()
                .ok_or_else(|| "'Session' must be a string".to_string())?;
            let sid = SessionId::from_str(s).map_err(|e| e.to_string())?;
            Ok(Some(ActorId::Session(sid)))
        }
        "Issue" => {
            let s = value
                .as_str()
                .ok_or_else(|| "'Issue' must be a string".to_string())?;
            let iid = IssueId::from_str(s).map_err(|e| e.to_string())?;
            Ok(Some(ActorId::Issue(iid)))
        }
        "Service" => {
            let s = value
                .as_str()
                .ok_or_else(|| "'Service' must be a string".to_string())?;
            Ok(Some(ActorId::Service(s.to_string())))
        }
        "User" => {
            let name = str_field(value, "name")?;
            let u = Username::try_new(&name).map_err(|e| e.to_string())?;
            Ok(Some(ActorId::User(u)))
        }
        "Agent" => {
            let name = str_field(value, "name")?;
            let a = AgentName::try_new(&name).map_err(|e| e.to_string())?;
            Ok(Some(ActorId::Agent(a)))
        }
        "Adhoc" => {
            let sid_str = str_field(value, "session_id")?;
            let sid = SessionId::from_str(&sid_str).map_err(|e| e.to_string())?;
            Ok(Some(ActorId::Adhoc(sid)))
        }
        "External" => {
            let system_str = str_field(value, "system")?;
            let username = str_field(value, "username")?;
            let sys = ExternalSystem::try_new(&system_str).map_err(|e| e.to_string())?;
            if username.is_empty() {
                return Err("'username' must not be empty".to_string());
            }
            Ok(Some(ActorId::External {
                system: sys,
                username,
            }))
        }
        _ => Ok(None),
    }
}

fn entries_to_legacy_string(entries: &[(String, serde_json::Value)]) -> String {
    let map: serde_json::Map<String, serde_json::Value> = entries.iter().cloned().collect();
    serde_json::Value::Object(map).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn actor_ref_serialization_round_trip_authenticated() {
        let actor_ref = ActorRef::Authenticated {
            actor_id: ActorId::Username(Username::from("bob")),
            session_id: None,
        };
        let json = serde_json::to_string(&actor_ref).unwrap();
        let deserialized: ActorRef = serde_json::from_str(&json).unwrap();
        assert_eq!(actor_ref, deserialized);
    }

    #[test]
    fn actor_ref_serialization_round_trip_system() {
        let actor_ref = ActorRef::System {
            worker_name: "task-spawner".into(),
            on_behalf_of: Some(ActorId::Username(Username::from("carol"))),
        };
        let json = serde_json::to_string(&actor_ref).unwrap();
        let deserialized: ActorRef = serde_json::from_str(&json).unwrap();
        assert_eq!(actor_ref, deserialized);
    }

    #[test]
    fn actor_ref_serialization_round_trip_automation() {
        let actor_ref = ActorRef::Automation {
            automation_name: "cascade_issue_status".into(),
            triggered_by: Some(Box::new(ActorRef::Authenticated {
                actor_id: ActorId::Username(Username::from("dave")),
                session_id: None,
            })),
        };
        let json = serde_json::to_string(&actor_ref).unwrap();
        let deserialized: ActorRef = serde_json::from_str(&json).unwrap();
        assert_eq!(actor_ref, deserialized);
    }

    #[test]
    fn actor_ref_display_name_authenticated() {
        let actor_ref = ActorRef::Authenticated {
            actor_id: ActorId::Username(Username::from("alice")),
            session_id: None,
        };
        assert_eq!(actor_ref.display_name(), "alice");
    }

    #[test]
    fn actor_ref_display_name_system_with_on_behalf_of() {
        let actor_ref = ActorRef::System {
            worker_name: "task-spawner".into(),
            on_behalf_of: Some(ActorId::Username(Username::from("bob"))),
        };
        assert_eq!(actor_ref.display_name(), "task-spawner (on behalf of bob)");
    }

    #[test]
    fn actor_ref_display_name_system_without_on_behalf_of() {
        let actor_ref = ActorRef::System {
            worker_name: "background".into(),
            on_behalf_of: None,
        };
        assert_eq!(actor_ref.display_name(), "background");
    }

    #[test]
    fn actor_ref_display_name_automation() {
        let actor_ref = ActorRef::Automation {
            automation_name: "cascade".into(),
            triggered_by: Some(Box::new(ActorRef::Authenticated {
                actor_id: ActorId::Username(Username::from("eve")),
                session_id: None,
            })),
        };
        assert_eq!(actor_ref.display_name(), "cascade (triggered by eve)");
    }

    #[test]
    fn actor_ref_test_helper() {
        let actor_ref = ActorRef::test();
        assert_eq!(
            actor_ref,
            ActorRef::System {
                worker_name: "test".into(),
                on_behalf_of: None,
            }
        );
    }

    #[test]
    fn parse_actor_name_user() {
        let result = parse_actor_name("u-alice");
        assert_eq!(result, Some(ActorId::Username(Username::from("alice"))));
    }

    #[test]
    fn parse_actor_name_task() {
        let task_id = SessionId::from_str("s-abcdef").unwrap();
        let result = parse_actor_name("w-s-abcdef");
        assert_eq!(result, Some(ActorId::Session(task_id)));
    }

    #[test]
    fn actor_id_issue_serialization_round_trip() {
        let issue_id = IssueId::from_str("i-abcdef").unwrap();
        let actor_id = ActorId::Issue(issue_id);
        let json = serde_json::to_string(&actor_id).unwrap();
        let deserialized: ActorId = serde_json::from_str(&json).unwrap();
        assert_eq!(actor_id, deserialized);
    }

    #[test]
    fn parse_actor_name_issue() {
        let issue_id = IssueId::from_str("i-abcdef").unwrap();
        let result = parse_actor_name("a-i-abcdef");
        assert_eq!(result, Some(ActorId::Issue(issue_id)));
    }

    #[test]
    fn parse_actor_name_empty_issue() {
        assert_eq!(parse_actor_name("a-"), None);
    }

    #[test]
    fn actor_ref_display_name_issue() {
        let issue_id = IssueId::from_str("i-abcdef").unwrap();
        let actor_ref = ActorRef::Authenticated {
            actor_id: ActorId::Issue(issue_id),
            session_id: None,
        };
        assert_eq!(actor_ref.display_name(), "i-abcdef");
    }

    #[test]
    fn parse_actor_name_empty_username() {
        assert_eq!(parse_actor_name("u-"), None);
    }

    #[test]
    fn parse_actor_name_empty_task() {
        assert_eq!(parse_actor_name("w-"), None);
    }

    #[test]
    fn parse_actor_name_invalid_prefix() {
        assert_eq!(parse_actor_name("x-123"), None);
    }

    #[test]
    fn actor_id_display_username() {
        let actor_id = ActorId::Username(Username::from("alice"));
        assert_eq!(actor_id.to_string(), "u-alice");
    }

    #[test]
    fn actor_id_display_task() {
        let task_id = SessionId::from_str("s-abcdef").unwrap();
        let actor_id = ActorId::Session(task_id);
        assert_eq!(actor_id.to_string(), "w-s-abcdef");
    }

    #[test]
    fn actor_id_display_issue() {
        let issue_id = IssueId::from_str("i-abcdef").unwrap();
        let actor_id = ActorId::Issue(issue_id);
        assert_eq!(actor_id.to_string(), "a-i-abcdef");
    }

    #[test]
    fn actor_id_from_str_issue_id() {
        let actor: ActorId = "i-abcdef".parse().unwrap();
        match actor {
            ActorId::Issue(id) => assert_eq!(id.to_string(), "i-abcdef"),
            other => panic!("expected ActorId::Issue, got {other:?}"),
        }
    }

    #[test]
    fn actor_id_from_str_task_id() {
        let actor: ActorId = "s-abcdef".parse().unwrap();
        match actor {
            ActorId::Session(id) => assert_eq!(id.to_string(), "s-abcdef"),
            other => panic!("expected ActorId::Session, got {other:?}"),
        }
    }

    #[test]
    fn actor_id_from_str_username() {
        let actor: ActorId = "alice".parse().unwrap();
        match actor {
            ActorId::Username(username) => assert_eq!(username.as_str(), "alice"),
            other => panic!("expected ActorId::Username, got {other:?}"),
        }
    }

    #[test]
    fn actor_id_from_str_empty_fails() {
        assert!("".parse::<ActorId>().is_err());
        assert!("  ".parse::<ActorId>().is_err());
    }

    #[test]
    fn actor_id_from_str_trims_whitespace() {
        let actor: ActorId = "  bob  ".parse().unwrap();
        match actor {
            ActorId::Username(username) => assert_eq!(username.as_str(), "bob"),
            other => panic!("expected ActorId::Username, got {other:?}"),
        }
    }

    #[test]
    fn try_from_actor_identity_user() {
        let identity = ActorIdentity::User {
            username: Username::from("alice"),
        };
        let actor_id = ActorId::try_from(identity).unwrap();
        assert_eq!(actor_id, ActorId::Username(Username::from("alice")));
    }

    #[test]
    fn try_from_actor_identity_task() {
        let task_id = SessionId::from_str("s-abcdef").unwrap();
        let identity = ActorIdentity::Session {
            session_id: task_id.clone(),
            creator: Username::from("bob"),
        };
        let actor_id = ActorId::try_from(identity).unwrap();
        assert_eq!(actor_id, ActorId::Session(task_id));
    }

    #[test]
    fn try_from_actor_identity_issue() {
        let issue_id = IssueId::from_str("i-abcdef").unwrap();
        let identity = ActorIdentity::Issue {
            issue_id: issue_id.clone(),
            creator: Username::from("carol"),
        };
        let actor_id = ActorId::try_from(identity).unwrap();
        assert_eq!(actor_id, ActorId::Issue(issue_id));
    }

    #[test]
    fn actor_id_display_service() {
        let actor_id = ActorId::Service("bff".to_string());
        assert_eq!(actor_id.to_string(), "svc-bff");
    }

    #[test]
    fn parse_actor_name_service() {
        let result = parse_actor_name("svc-bff");
        assert_eq!(result, Some(ActorId::Service("bff".to_string())));
    }

    #[test]
    fn parse_actor_name_empty_service() {
        assert_eq!(parse_actor_name("svc-"), None);
    }

    #[test]
    fn actor_id_from_str_service() {
        let actor: ActorId = "svc-bff".parse().unwrap();
        match actor {
            ActorId::Service(name) => assert_eq!(name, "bff"),
            other => panic!("expected ActorId::Service, got {other:?}"),
        }
    }

    #[test]
    fn actor_id_from_str_service_empty_fails() {
        assert!("svc-".parse::<ActorId>().is_err());
    }

    #[test]
    fn try_from_actor_identity_service() {
        let identity = ActorIdentity::Service {
            service_name: "bff".to_string(),
        };
        let actor_id = ActorId::try_from(identity).unwrap();
        assert_eq!(actor_id, ActorId::Service("bff".to_string()));
    }

    #[test]
    fn actor_id_service_serialization_round_trip() {
        let actor_id = ActorId::Service("bff".to_string());
        let json = serde_json::to_string(&actor_id).unwrap();
        let deserialized: ActorId = serde_json::from_str(&json).unwrap();
        assert_eq!(actor_id, deserialized);
    }

    #[test]
    fn actor_ref_display_name_service() {
        let actor_ref = ActorRef::Authenticated {
            actor_id: ActorId::Service("bff".to_string()),
            session_id: None,
        };
        assert_eq!(actor_ref.display_name(), "svc-bff");
    }

    #[test]
    fn on_behalf_of_authenticated_returns_actor_id() {
        let actor_ref = ActorRef::Authenticated {
            actor_id: ActorId::Username(Username::from("alice")),
            session_id: None,
        };
        assert_eq!(
            actor_ref.on_behalf_of(),
            Some(ActorId::Username(Username::from("alice")))
        );
    }

    #[test]
    fn on_behalf_of_system_returns_on_behalf_of_actor() {
        let actor_ref = ActorRef::System {
            worker_name: "task-spawner".into(),
            on_behalf_of: Some(ActorId::Username(Username::from("bob"))),
        };
        assert_eq!(
            actor_ref.on_behalf_of(),
            Some(ActorId::Username(Username::from("bob")))
        );
    }

    #[test]
    fn on_behalf_of_system_without_principal_returns_none() {
        let actor_ref = ActorRef::System {
            worker_name: "background".into(),
            on_behalf_of: None,
        };
        assert_eq!(actor_ref.on_behalf_of(), None);
    }

    #[test]
    fn on_behalf_of_automation_unwraps_triggered_by() {
        let actor_ref = ActorRef::Automation {
            automation_name: "github_pr_sync".into(),
            triggered_by: Some(Box::new(ActorRef::Authenticated {
                actor_id: ActorId::Username(Username::from("carol")),
                session_id: None,
            })),
        };
        assert_eq!(
            actor_ref.on_behalf_of(),
            Some(ActorId::Username(Username::from("carol")))
        );
    }

    #[test]
    fn on_behalf_of_automation_recurses_through_nested_automations() {
        // Automation -> Automation -> Authenticated(dave)
        let inner = ActorRef::Automation {
            automation_name: "github_pr_sync".into(),
            triggered_by: Some(Box::new(ActorRef::Authenticated {
                actor_id: ActorId::Username(Username::from("dave")),
                session_id: None,
            })),
        };
        let outer = ActorRef::Automation {
            automation_name: "link_conversation_to_artifacts".into(),
            triggered_by: Some(Box::new(inner)),
        };
        assert_eq!(
            outer.on_behalf_of(),
            Some(ActorId::Username(Username::from("dave")))
        );
    }

    #[test]
    fn on_behalf_of_automation_recurses_into_system_on_behalf_of() {
        // Automation -> System(on_behalf_of=session)
        let session_id = SessionId::from_str("s-abcdef").unwrap();
        let actor_ref = ActorRef::Automation {
            automation_name: "github_pr_sync".into(),
            triggered_by: Some(Box::new(ActorRef::System {
                worker_name: "task-spawner".into(),
                on_behalf_of: Some(ActorId::Session(session_id.clone())),
            })),
        };
        assert_eq!(actor_ref.on_behalf_of(), Some(ActorId::Session(session_id)));
    }

    #[test]
    fn on_behalf_of_automation_without_trigger_returns_none() {
        let actor_ref = ActorRef::Automation {
            automation_name: "standalone".into(),
            triggered_by: None,
        };
        assert_eq!(actor_ref.on_behalf_of(), None);
    }

    // -----------------------------------------------------------------
    // Phase-1 additions
    // -----------------------------------------------------------------

    fn alice_user() -> ActorId {
        ActorId::User(Username::try_new("alice").unwrap())
    }
    fn swe_agent() -> ActorId {
        ActorId::Agent(AgentName::try_new("swe").unwrap())
    }
    fn adhoc_session() -> ActorId {
        ActorId::Adhoc(SessionId::from_str("s-abcdef").unwrap())
    }
    fn gh_external() -> ActorId {
        ActorId::External {
            system: ExternalSystem::try_new("github").unwrap(),
            username: "jayantk".to_string(),
        }
    }

    // --- Display for new variants ---

    #[test]
    fn actor_id_display_user_path_form() {
        assert_eq!(alice_user().to_string(), "users/alice");
    }

    #[test]
    fn actor_id_display_agent_path_form() {
        assert_eq!(swe_agent().to_string(), "agents/swe");
    }

    #[test]
    fn actor_id_display_adhoc_path_form() {
        assert_eq!(adhoc_session().to_string(), "adhoc/s-abcdef");
    }

    #[test]
    fn actor_id_display_external_path_form() {
        assert_eq!(gh_external().to_string(), "external/github/jayantk");
    }

    #[test]
    fn actor_id_display_legacy_round_trips_raw_string() {
        let raw = "unknown-blob-format".to_string();
        assert_eq!(ActorId::Legacy(raw.clone()).to_string(), raw);
    }

    // --- FromStr for new variants (NOT sessions/) ---

    #[test]
    fn actor_id_from_str_user_path() {
        let id: ActorId = "users/alice".parse().unwrap();
        assert_eq!(id, alice_user());
    }

    #[test]
    fn actor_id_from_str_agent_path() {
        let id: ActorId = "agents/swe".parse().unwrap();
        assert_eq!(id, swe_agent());
    }

    #[test]
    fn actor_id_from_str_adhoc_path_returns_adhoc_not_session() {
        let id: ActorId = "adhoc/s-abcdef".parse().unwrap();
        assert!(
            matches!(id, ActorId::Adhoc(_)),
            "adhoc/<x> must parse to ActorId::Adhoc, not Session"
        );
    }

    #[test]
    fn actor_id_from_str_does_not_recognize_sessions_path() {
        // Clarification C2: the canonical path is adhoc/, NOT sessions/.
        // "sessions/x" falls through to Username with a leading "sessions/"
        // segment, which Username::from accepts as a free string today —
        // but it must NOT parse as Session or Adhoc.
        let id: ActorId = "sessions/x".parse().unwrap();
        assert!(
            !matches!(id, ActorId::Session(_) | ActorId::Adhoc(_)),
            "sessions/<x> is NOT a recognised actor path (got: {id:?})"
        );
    }

    #[test]
    fn actor_id_from_str_external_path() {
        let id: ActorId = "external/github/jayantk".parse().unwrap();
        assert_eq!(id, gh_external());
    }

    #[test]
    fn actor_id_from_str_external_missing_username_errors() {
        assert!("external/github".parse::<ActorId>().is_err());
        assert!("external/github/".parse::<ActorId>().is_err());
    }

    #[test]
    fn actor_id_from_str_invalid_user_path_errors() {
        assert!("users/".parse::<ActorId>().is_err());
        assert!("users/has space".parse::<ActorId>().is_err());
    }

    // --- Serde for new variants ---

    #[test]
    fn actor_id_user_serde_round_trip() {
        let id = alice_user();
        let value = serde_json::to_value(&id).unwrap();
        assert_eq!(value, json!({"User": {"name": "alice"}}));
        let back: ActorId = serde_json::from_value(value).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn actor_id_agent_serde_round_trip() {
        let id = swe_agent();
        let value = serde_json::to_value(&id).unwrap();
        assert_eq!(value, json!({"Agent": {"name": "swe"}}));
        let back: ActorId = serde_json::from_value(value).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn actor_id_adhoc_serde_round_trip() {
        let id = adhoc_session();
        let value = serde_json::to_value(&id).unwrap();
        assert_eq!(value, json!({"Adhoc": {"session_id": "s-abcdef"}}));
        let back: ActorId = serde_json::from_value(value).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn actor_id_external_serde_round_trip() {
        let id = gh_external();
        let value = serde_json::to_value(&id).unwrap();
        assert_eq!(
            value,
            json!({"External": {"system": "github", "username": "jayantk"}})
        );
        let back: ActorId = serde_json::from_value(value).unwrap();
        assert_eq!(back, id);
    }

    // --- Legacy fallback ---

    #[test]
    fn actor_id_legacy_string_round_trip() {
        let id = ActorId::Legacy("free-form-legacy-blob".to_string());
        let value = serde_json::to_value(&id).unwrap();
        assert_eq!(value, json!("free-form-legacy-blob"));
        let back: ActorId = serde_json::from_value(value).unwrap();
        assert_eq!(back, id);
    }

    // The `Legacy` variant is gated on a release-soak with zero
    // `actor_id_legacy_decode` warn-logs (design §10 Q3, §8 C3). The
    // workspace has no `tracing-test` dep, so this is a smoke test that
    // both bare-string deserialization paths (`visit_str` /
    // `visit_string`) run through the warn! without panicking and yield
    // `ActorId::Legacy(...)`. Drop alongside the variant removal.
    #[test]
    fn actor_id_legacy_decode_smoke_emits_warn_without_panic() {
        // visit_str path (borrowed): `from_str` / `from_slice` hand the
        // visitor a `&str`.
        let id: ActorId = serde_json::from_str("\"jayantk\"").unwrap();
        assert_eq!(id, ActorId::Legacy("jayantk".to_string()));

        // visit_string path (owned): `from_value(Value::String(_))` hands
        // the visitor an owned `String`.
        let id: ActorId = serde_json::from_value(json!("agents/swe")).unwrap();
        assert_eq!(id, ActorId::Legacy("agents/swe".to_string()));
    }

    #[test]
    fn actor_id_unknown_tagged_falls_back_to_legacy() {
        // Unknown externally-tagged form — captured losslessly as Legacy(raw JSON).
        let payload = json!({"Robot": {"name": "r2"}});
        let id: ActorId = serde_json::from_value(payload.clone()).unwrap();
        let raw = match &id {
            ActorId::Legacy(raw) => raw.clone(),
            other => panic!("expected Legacy, got {other:?}"),
        };
        let reparsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(reparsed, payload);
    }

    #[test]
    fn actor_id_unknown_externally_tagged_falls_back_to_legacy() {
        let payload = json!({"Spaceship": "enterprise"});
        let id: ActorId = serde_json::from_value(payload.clone()).unwrap();
        assert!(matches!(id, ActorId::Legacy(_)), "got {id:?}");
    }

    #[test]
    fn actor_id_unknown_map_shape_falls_back_to_legacy() {
        // Multiple keys — not a single-entry externally-tagged map.
        let payload = json!({"a": 1, "b": 2});
        let id: ActorId = serde_json::from_value(payload).unwrap();
        assert!(matches!(id, ActorId::Legacy(_)), "got {id:?}");
    }

    #[test]
    fn actor_id_old_internally_tagged_form_falls_back_to_legacy() {
        // The pre-Phase-1-cleanup wire form (`{"kind": "user", ...}`) is
        // no longer a recognised ActorId shape. It has multiple keys so
        // it isn't a single-entry external-tag map, and lands in Legacy.
        let payload = json!({"kind": "user", "name": "alice"});
        let id: ActorId = serde_json::from_value(payload).unwrap();
        assert!(matches!(id, ActorId::Legacy(_)), "got {id:?}");
    }

    // --- Existing externally-tagged wire format still round-trips ---

    #[test]
    fn actor_id_legacy_external_username_still_deserializes() {
        let payload = json!({"Username": "alice"});
        let id: ActorId = serde_json::from_value(payload.clone()).unwrap();
        assert_eq!(id, ActorId::Username(Username::from("alice")));
        let value = serde_json::to_value(&id).unwrap();
        assert_eq!(value, payload);
    }

    #[test]
    fn actor_id_legacy_external_session_still_deserializes() {
        let payload = json!({"Session": "s-abcdef"});
        let id: ActorId = serde_json::from_value(payload.clone()).unwrap();
        assert_eq!(
            id,
            ActorId::Session(SessionId::from_str("s-abcdef").unwrap())
        );
        let value = serde_json::to_value(&id).unwrap();
        assert_eq!(value, payload);
    }

    // --- Distinctness of Username vs User: wire form picks the typed one ---

    #[test]
    fn user_and_username_variants_are_distinct() {
        let typed = alice_user();
        let legacy = ActorId::Username(Username::from("alice"));
        assert_ne!(typed, legacy);
    }

    #[test]
    fn typed_external_form_deserializes_to_user_not_username() {
        // The wire form `{"User": {...}}` is unambiguous: it must
        // produce ActorId::User, never ActorId::Username — the legacy
        // externally-tagged form is `{"Username": "..."}`.
        let id: ActorId = serde_json::from_value(json!({"User": {"name": "alice"}})).unwrap();
        assert_eq!(id, alice_user());
        assert!(!matches!(id, ActorId::Username(_)));
    }

    // --- Display name for new variants ---

    #[test]
    fn actor_ref_display_name_user() {
        let r = ActorRef::Authenticated {
            actor_id: alice_user(),
            session_id: None,
        };
        assert_eq!(r.display_name(), "alice");
    }

    #[test]
    fn actor_ref_display_name_agent() {
        let r = ActorRef::Authenticated {
            actor_id: swe_agent(),
            session_id: None,
        };
        assert_eq!(r.display_name(), "swe");
    }

    #[test]
    fn actor_ref_display_name_external() {
        let r = ActorRef::Authenticated {
            actor_id: gh_external(),
            session_id: None,
        };
        assert_eq!(r.display_name(), "external/github/jayantk");
    }

    // -----------------------------------------------------------------
    // Phase-2: parse_actor_name accepts the canonical path forms used
    // by `Actor::name()` once `create_actor_for_job` mints `Agent` /
    // `Adhoc` tokens. The auth layer round-trips `actor.name()` →
    // `AuthToken::parse(...)` → `Actor::parse_name(...)`, so the new
    // display forms must parse back.
    // -----------------------------------------------------------------

    #[test]
    fn parse_actor_name_agent_path() {
        let result = parse_actor_name("agents/swe");
        assert_eq!(
            result,
            Some(ActorId::Agent(AgentName::try_new("swe").unwrap()))
        );
    }

    #[test]
    fn parse_actor_name_adhoc_path() {
        let sid = SessionId::from_str("s-abcdef").unwrap();
        let result = parse_actor_name("adhoc/s-abcdef");
        assert_eq!(result, Some(ActorId::Adhoc(sid)));
    }

    #[test]
    fn parse_actor_name_user_path() {
        let result = parse_actor_name("users/alice");
        assert_eq!(
            result,
            Some(ActorId::User(Username::try_new("alice").unwrap()))
        );
    }

    #[test]
    fn parse_actor_name_external_path() {
        let result = parse_actor_name("external/github/jayantk");
        assert_eq!(
            result,
            Some(ActorId::External {
                system: ExternalSystem::try_new("github").unwrap(),
                username: "jayantk".to_string(),
            })
        );
    }

    #[test]
    fn parse_actor_name_external_missing_username_returns_none() {
        assert_eq!(parse_actor_name("external/github"), None);
        assert_eq!(parse_actor_name("external/github/"), None);
    }

    #[test]
    fn parse_actor_name_agent_empty_returns_none() {
        assert_eq!(parse_actor_name("agents/"), None);
    }

    #[test]
    fn parse_actor_name_round_trips_with_display_for_agent() {
        let id = swe_agent();
        let parsed = parse_actor_name(&id.to_string()).expect("agent display must round-trip");
        assert_eq!(parsed, id);
    }

    #[test]
    fn parse_actor_name_round_trips_with_display_for_adhoc() {
        let id = adhoc_session();
        let parsed = parse_actor_name(&id.to_string()).expect("adhoc display must round-trip");
        assert_eq!(parsed, id);
    }

    // -----------------------------------------------------------------
    // Phase 3a: ActorRef::Authenticated.session_id round-trip + §5.4
    // back-compat contract — JSON blobs without the field must
    // deserialize as `session_id: None`.
    // -----------------------------------------------------------------

    #[test]
    fn actor_ref_authenticated_with_session_id_round_trips() {
        let sid = SessionId::from_str("s-abcdef").unwrap();
        let actor_ref = ActorRef::Authenticated {
            actor_id: ActorId::Agent(AgentName::try_new("swe").unwrap()),
            session_id: Some(sid),
        };
        let json = serde_json::to_string(&actor_ref).unwrap();
        let deserialized: ActorRef = serde_json::from_str(&json).unwrap();
        assert_eq!(actor_ref, deserialized);
    }

    #[test]
    fn actor_ref_authenticated_with_none_session_id_round_trips() {
        let actor_ref = ActorRef::Authenticated {
            actor_id: ActorId::Username(Username::from("alice")),
            session_id: None,
        };
        let json = serde_json::to_string(&actor_ref).unwrap();
        let deserialized: ActorRef = serde_json::from_str(&json).unwrap();
        assert_eq!(actor_ref, deserialized);
    }

    #[test]
    fn originating_session_id_authenticated_agent_with_sid() {
        let sid = SessionId::from_str("s-abcdef").unwrap();
        let actor_ref = ActorRef::Authenticated {
            actor_id: ActorId::Agent(AgentName::try_new("swe").unwrap()),
            session_id: Some(sid.clone()),
        };
        assert_eq!(actor_ref.originating_session_id(), Some(&sid));
    }

    #[test]
    fn originating_session_id_authenticated_agent_without_sid() {
        let actor_ref = ActorRef::Authenticated {
            actor_id: ActorId::Agent(AgentName::try_new("swe").unwrap()),
            session_id: None,
        };
        assert_eq!(actor_ref.originating_session_id(), None);
    }

    #[test]
    fn originating_session_id_authenticated_username_without_sid() {
        let actor_ref = ActorRef::Authenticated {
            actor_id: ActorId::Username(Username::from("alice")),
            session_id: None,
        };
        assert_eq!(actor_ref.originating_session_id(), None);
    }

    #[test]
    fn originating_session_id_automation_recurses_through_triggered_by() {
        let sid = SessionId::from_str("s-abcdef").unwrap();
        let actor_ref = ActorRef::Automation {
            automation_name: "github_pr_sync".into(),
            triggered_by: Some(Box::new(ActorRef::Authenticated {
                actor_id: ActorId::Agent(AgentName::try_new("swe").unwrap()),
                session_id: Some(sid.clone()),
            })),
        };
        assert_eq!(actor_ref.originating_session_id(), Some(&sid));
    }

    #[test]
    fn originating_session_id_system_returns_none() {
        let actor_ref = ActorRef::System {
            worker_name: "task-spawner".into(),
            on_behalf_of: Some(ActorId::Agent(AgentName::try_new("swe").unwrap())),
        };
        assert_eq!(actor_ref.originating_session_id(), None);
    }

    #[test]
    fn actor_ref_authenticated_without_field_deserializes_to_none() {
        // Pre-Phase-3a Versioned<T>.actor blobs predate session_id.
        // Per design §5.4, deserializing such a blob must yield
        // `session_id: None`.
        let legacy = json!({
            "Authenticated": { "actor_id": { "Username": "alice" } }
        });
        let deserialized: ActorRef = serde_json::from_value(legacy).unwrap();
        assert_eq!(
            deserialized,
            ActorRef::Authenticated {
                actor_id: ActorId::Username(Username::from("alice")),
                session_id: None,
            }
        );
    }
}
