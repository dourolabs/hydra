use crate::api::v1::agents::AgentName;
use crate::api::v1::users::Username;
use crate::ids::{SessionId, TriggerId};
use crate::principal::ExternalSystem;
use crate::whoami::ActorIdentity;
use serde::de::{Error as DeError, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// Typed identifier for the principal that performed an operation.
///
/// Variants:
/// - **`User(Username)`** — a human user.
/// - **`Agent(AgentName)`** — first-class named agent (e.g. `pm`, `swe`).
/// - **`Adhoc(SessionId)`** — a session created outside the agent
///   system. Canonical path is `adhoc/<session-id>` (clarification C2
///   in `/designs/actor-system-overhaul.md` — *not* `sessions/`).
/// - **`External { system, username }`** — an identity that lives in an
///   external system (e.g. GitHub) and has no corresponding Hydra user.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
// ActorId has a hand-rolled serde impl (below) that emits the
// externally-tagged form for every variant. ts-rs derives the
// externally-tagged union for us. The referenced newtypes (`Username`,
// `AgentName`, `SessionId`, `ExternalSystem`) are all `type X = string;`
// aliases today, so plain `string` is the accurate TS shape. The Rust
// serde tests in this file pin the wire form on the Rust side; the
// matching TS-side fixture test lives in
// `hydra-web/packages/web/src/utils/actors.test.ts`.
#[cfg_attr(
    feature = "ts",
    ts(
        export,
        type = "\
| { User: { name: string } } \
| { Agent: { name: string } } \
| { Adhoc: { session_id: string } } \
| { External: { system: string; username: string } }"
    )
)]
pub enum ActorId {
    User(Username),
    Agent(AgentName),
    Adhoc(SessionId),
    External {
        system: ExternalSystem,
        username: String,
    },
}

impl ActorId {
    /// Human-readable display name used by [`ActorRef::display_name`].
    pub fn display_name(&self) -> String {
        match self {
            ActorId::User(u) => u.as_str().to_string(),
            ActorId::Agent(a) => a.as_str().to_string(),
            ActorId::Adhoc(s) => s.to_string(),
            ActorId::External { system, username } => {
                format!("external/{}/{}", system.as_str(), username)
            }
        }
    }
}

impl fmt::Display for ActorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActorId::User(u) => write!(f, "users/{}", u.as_str()),
            ActorId::Agent(a) => write!(f, "agents/{}", a.as_str()),
            ActorId::Adhoc(s) => write!(f, "adhoc/{s}"),
            ActorId::External { system, username } => {
                write!(f, "external/{}/{}", system.as_str(), username)
            }
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
    Trigger {
        trigger_id: TriggerId,
        on_behalf_of: Option<ActorId>,
    },
}

impl ActorRef {
    /// Human-readable display name for this actor reference.
    pub fn display_name(&self) -> String {
        match self {
            ActorRef::Authenticated { actor_id, .. } => actor_id.display_name(),
            ActorRef::System {
                worker_name,
                on_behalf_of,
            } => {
                if let Some(behalf) = on_behalf_of {
                    let behalf_name = behalf.display_name();
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
            ActorRef::Trigger {
                trigger_id,
                on_behalf_of,
            } => {
                if let Some(behalf) = on_behalf_of {
                    let behalf_name = behalf.display_name();
                    format!("{trigger_id} (on behalf of {behalf_name})")
                } else {
                    trigger_id.to_string()
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
    /// `Automation -> Automation -> Authenticated(User("alice"))` resolves to
    /// `User("alice")`.
    pub fn on_behalf_of(&self) -> Option<ActorId> {
        match self {
            ActorRef::Authenticated { actor_id, .. } => Some(actor_id.clone()),
            ActorRef::System { on_behalf_of, .. } => on_behalf_of.clone(),
            ActorRef::Automation { triggered_by, .. } => {
                triggered_by.as_ref().and_then(|t| t.on_behalf_of())
            }
            ActorRef::Trigger { on_behalf_of, .. } => on_behalf_of.clone(),
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
            ActorRef::Trigger { .. } => None,
        }
    }
}

/// Parse a canonical path-form string into an `ActorId`.
///
/// Recognised forms (design §3.3):
/// `users/<x>`, `agents/<x>`, `adhoc/<x>`, `external/<sys>/<x>`.
impl FromStr for ActorId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err("actor ID must not be empty".to_string());
        }

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

        Err(format!(
            "unrecognized actor id '{trimmed}' (expected one of: users/<x>, agents/<x>, adhoc/<x>, external/<sys>/<x>)"
        ))
    }
}

impl TryFrom<ActorIdentity> for ActorId {
    type Error = String;

    fn try_from(identity: ActorIdentity) -> Result<Self, Self::Error> {
        #[allow(unreachable_patterns)]
        match identity {
            ActorIdentity::User { username } => Ok(ActorId::User(username)),
            ActorIdentity::Agent { name, .. } => Ok(ActorId::Agent(name)),
            ActorIdentity::Adhoc { session_id, .. } => Ok(ActorId::Adhoc(session_id)),
            _ => Err("unsupported actor identity type".to_string()),
        }
    }
}

/// Parse an actor name string in canonical path form
/// (`users/<x>`, `agents/<x>`, `adhoc/<x>`, `external/<sys>/<x>`) into an
/// `ActorId`. Returns `None` if the name does not match a recognised form
/// or is otherwise invalid.
///
/// The auth layer's `AuthToken::parse` calls this on the token's actor
/// prefix; the prefix is whatever `Actor::name()` produces (which is
/// `ActorId::to_string()`), so this function must accept everything
/// `Display` emits.
pub fn parse_actor_name(name: &str) -> Option<ActorId> {
    ActorId::from_str(name).ok()
}

// -------------------------------------------------------------------------
// Serde — bespoke impls.
//
// Wire format (externally-tagged):
//   {"User":     {"name":       "alice"}}
//   {"Agent":    {"name":       "swe"}}
//   {"Adhoc":    {"session_id": "s-..."}}
//   {"External": {"system":     "github", "username": "jayantk"}}
//
// Any payload the deserializer can't interpret is an error. The
// `actor_variant_cleanup` migration rewrites every stored pre-cleanup
// shape (`{"Username":...}`, `{"Session":...}`, `{"Issue":...}`,
// `{"Service":...}`, bare strings, multi-key maps) into one of the four
// recognised tags above so production rows survive the variant deletion.
// -------------------------------------------------------------------------

impl Serialize for ActorId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut m = serializer.serialize_map(Some(1))?;
        match self {
            ActorId::User(u) => {
                m.serialize_entry("User", &serde_json::json!({ "name": u.as_str() }))?;
            }
            ActorId::Agent(a) => {
                m.serialize_entry("Agent", &serde_json::json!({ "name": a.as_str() }))?;
            }
            ActorId::Adhoc(s) => {
                m.serialize_entry("Adhoc", &serde_json::json!({ "session_id": s.to_string() }))?;
            }
            ActorId::External { system, username } => {
                m.serialize_entry(
                    "External",
                    &serde_json::json!({
                        "system": system.as_str(),
                        "username": username,
                    }),
                )?;
            }
        }
        m.end()
    }
}

impl<'de> Deserialize<'de> for ActorId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(ActorIdVisitor)
    }
}

struct ActorIdVisitor;

impl<'de> Visitor<'de> for ActorIdVisitor {
    type Value = ActorId;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an externally-tagged ActorId map")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries: Vec<(String, serde_json::Value)> = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            let value: serde_json::Value = map.next_value()?;
            entries.push((key, value));
        }

        if entries.len() != 1 {
            return Err(A::Error::custom(format!(
                "expected exactly one externally-tagged ActorId entry, got {}",
                entries.len()
            )));
        }
        let (tag, value) = &entries[0];
        parse_external(tag, value).map_err(A::Error::custom)
    }
}

fn parse_external(tag: &str, value: &serde_json::Value) -> Result<ActorId, String> {
    let str_field = |obj: &serde_json::Value, name: &'static str| -> Result<String, String> {
        let v = obj
            .get(name)
            .ok_or_else(|| format!("missing field '{name}'"))?;
        v.as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| format!("field '{name}' must be a string"))
    };
    match tag {
        "User" => {
            let name = str_field(value, "name")?;
            let u = Username::try_new(&name).map_err(|e| e.to_string())?;
            Ok(ActorId::User(u))
        }
        "Agent" => {
            let name = str_field(value, "name")?;
            let a = AgentName::try_new(&name).map_err(|e| e.to_string())?;
            Ok(ActorId::Agent(a))
        }
        "Adhoc" => {
            let sid_str = str_field(value, "session_id")?;
            let sid = SessionId::from_str(&sid_str).map_err(|e| e.to_string())?;
            Ok(ActorId::Adhoc(sid))
        }
        "External" => {
            let system_str = str_field(value, "system")?;
            let username = str_field(value, "username")?;
            let sys = ExternalSystem::try_new(&system_str).map_err(|e| e.to_string())?;
            if username.is_empty() {
                return Err("'username' must not be empty".to_string());
            }
            Ok(ActorId::External {
                system: sys,
                username,
            })
        }
        other => Err(format!("unknown ActorId variant tag '{other}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    fn sample_trigger_id() -> TriggerId {
        TriggerId::from_str("t-abcdef").unwrap()
    }

    #[test]
    fn actor_ref_serialization_round_trip_authenticated() {
        let actor_ref = ActorRef::Authenticated {
            actor_id: alice_user(),
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
            on_behalf_of: Some(alice_user()),
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
                actor_id: alice_user(),
                session_id: None,
            })),
        };
        let json = serde_json::to_string(&actor_ref).unwrap();
        let deserialized: ActorRef = serde_json::from_str(&json).unwrap();
        assert_eq!(actor_ref, deserialized);
    }

    #[test]
    fn actor_ref_serialization_round_trip_trigger_with_on_behalf_of() {
        let actor_ref = ActorRef::Trigger {
            trigger_id: sample_trigger_id(),
            on_behalf_of: Some(alice_user()),
        };
        let json = serde_json::to_string(&actor_ref).unwrap();
        let deserialized: ActorRef = serde_json::from_str(&json).unwrap();
        assert_eq!(actor_ref, deserialized);
    }

    #[test]
    fn actor_ref_serialization_round_trip_trigger_without_on_behalf_of() {
        let actor_ref = ActorRef::Trigger {
            trigger_id: sample_trigger_id(),
            on_behalf_of: None,
        };
        let json = serde_json::to_string(&actor_ref).unwrap();
        let deserialized: ActorRef = serde_json::from_str(&json).unwrap();
        assert_eq!(actor_ref, deserialized);
    }

    #[test]
    fn actor_ref_display_name_authenticated() {
        let actor_ref = ActorRef::Authenticated {
            actor_id: alice_user(),
            session_id: None,
        };
        assert_eq!(actor_ref.display_name(), "alice");
    }

    #[test]
    fn actor_ref_display_name_system_with_on_behalf_of() {
        let actor_ref = ActorRef::System {
            worker_name: "task-spawner".into(),
            on_behalf_of: Some(alice_user()),
        };
        assert_eq!(
            actor_ref.display_name(),
            "task-spawner (on behalf of alice)"
        );
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
                actor_id: alice_user(),
                session_id: None,
            })),
        };
        assert_eq!(actor_ref.display_name(), "cascade (triggered by alice)");
    }

    #[test]
    fn actor_ref_display_name_trigger_with_on_behalf_of() {
        let actor_ref = ActorRef::Trigger {
            trigger_id: sample_trigger_id(),
            on_behalf_of: Some(alice_user()),
        };
        assert_eq!(actor_ref.display_name(), "t-abcdef (on behalf of alice)");
    }

    #[test]
    fn actor_ref_display_name_trigger_without_on_behalf_of() {
        let actor_ref = ActorRef::Trigger {
            trigger_id: sample_trigger_id(),
            on_behalf_of: None,
        };
        assert_eq!(actor_ref.display_name(), "t-abcdef");
    }

    // --- FromStr / parse_actor_name accept the canonical path forms ---

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
    fn actor_id_from_str_adhoc_path_returns_adhoc() {
        let id: ActorId = "adhoc/s-abcdef".parse().unwrap();
        assert!(matches!(id, ActorId::Adhoc(_)));
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

    #[test]
    fn actor_id_from_str_unrecognized_form_errors() {
        // No bare-string fallback anymore — only canonical path forms are accepted.
        assert!("alice".parse::<ActorId>().is_err());
        assert!("u-alice".parse::<ActorId>().is_err());
        assert!("s-abcdef".parse::<ActorId>().is_err());
        assert!("i-abcdef".parse::<ActorId>().is_err());
        assert!("svc-bff".parse::<ActorId>().is_err());
        assert!("sessions/x".parse::<ActorId>().is_err());
    }

    #[test]
    fn actor_id_from_str_empty_fails() {
        assert!("".parse::<ActorId>().is_err());
        assert!("  ".parse::<ActorId>().is_err());
    }

    #[test]
    fn parse_actor_name_legacy_shorthand_rejected() {
        // Pre-cleanup `u-`/`a-`/`w-`/`svc-` shorthand is gone.
        assert_eq!(parse_actor_name("u-alice"), None);
        assert_eq!(parse_actor_name("w-s-abcdef"), None);
        assert_eq!(parse_actor_name("a-i-abcdef"), None);
        assert_eq!(parse_actor_name("svc-bff"), None);
    }

    #[test]
    fn parse_actor_name_round_trips_with_display() {
        for id in [alice_user(), swe_agent(), adhoc_session(), gh_external()] {
            let parsed = parse_actor_name(&id.to_string()).expect("display form must round-trip");
            assert_eq!(parsed, id);
        }
    }

    // --- Serde ---

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

    #[test]
    fn actor_id_unknown_tag_fails_to_deserialize() {
        // Pre-cleanup variants (`Username`/`Session`/`Issue`/`Service`)
        // and unknown tags must fail. The `actor_variant_cleanup`
        // migration rewrites stored rows BEFORE the server ever
        // deserializes them.
        for raw in [
            json!({"Username": "alice"}),
            json!({"Session": "s-abcdef"}),
            json!({"Issue": "i-abcdef"}),
            json!({"Service": "bff"}),
            json!({"Robot": {"name": "r2"}}),
        ] {
            assert!(
                serde_json::from_value::<ActorId>(raw.clone()).is_err(),
                "expected deserialization to fail for {raw}",
            );
        }
    }

    #[test]
    fn actor_id_bare_string_fails_to_deserialize() {
        assert!(serde_json::from_str::<ActorId>("\"jayantk\"").is_err());
    }

    #[test]
    fn actor_id_multi_key_map_fails_to_deserialize() {
        let payload = json!({"a": 1, "b": 2});
        assert!(serde_json::from_value::<ActorId>(payload).is_err());
    }

    // --- ActorRef.session_id round-trip and §5.4 back-compat ---

    #[test]
    fn originating_session_id_authenticated_agent_with_sid() {
        let sid = SessionId::from_str("s-abcdef").unwrap();
        let actor_ref = ActorRef::Authenticated {
            actor_id: swe_agent(),
            session_id: Some(sid.clone()),
        };
        assert_eq!(actor_ref.originating_session_id(), Some(&sid));
    }

    #[test]
    fn originating_session_id_authenticated_agent_without_sid() {
        let actor_ref = ActorRef::Authenticated {
            actor_id: swe_agent(),
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
                actor_id: swe_agent(),
                session_id: Some(sid.clone()),
            })),
        };
        assert_eq!(actor_ref.originating_session_id(), Some(&sid));
    }

    #[test]
    fn originating_session_id_trigger_returns_none() {
        let with_behalf = ActorRef::Trigger {
            trigger_id: sample_trigger_id(),
            on_behalf_of: Some(swe_agent()),
        };
        assert_eq!(with_behalf.originating_session_id(), None);

        let without_behalf = ActorRef::Trigger {
            trigger_id: sample_trigger_id(),
            on_behalf_of: None,
        };
        assert_eq!(without_behalf.originating_session_id(), None);
    }

    #[test]
    fn on_behalf_of_trigger_returns_inner_actor_id() {
        let with_behalf = ActorRef::Trigger {
            trigger_id: sample_trigger_id(),
            on_behalf_of: Some(alice_user()),
        };
        assert_eq!(with_behalf.on_behalf_of(), Some(alice_user()));

        let without_behalf = ActorRef::Trigger {
            trigger_id: sample_trigger_id(),
            on_behalf_of: None,
        };
        assert_eq!(without_behalf.on_behalf_of(), None);
    }

    #[test]
    fn originating_session_id_system_returns_none() {
        let actor_ref = ActorRef::System {
            worker_name: "task-spawner".into(),
            on_behalf_of: Some(swe_agent()),
        };
        assert_eq!(actor_ref.originating_session_id(), None);
    }

    #[test]
    fn actor_ref_authenticated_without_field_deserializes_to_none() {
        // Pre-Phase-3a Versioned<T>.actor blobs predate session_id.
        // Per design §5.4, deserializing such a blob must yield
        // `session_id: None`.
        let legacy = json!({
            "Authenticated": { "actor_id": { "User": {"name": "alice"} } }
        });
        let deserialized: ActorRef = serde_json::from_value(legacy).unwrap();
        assert_eq!(
            deserialized,
            ActorRef::Authenticated {
                actor_id: alice_user(),
                session_id: None,
            }
        );
    }

    // --- on_behalf_of ---

    #[test]
    fn on_behalf_of_authenticated_returns_actor_id() {
        let actor_ref = ActorRef::Authenticated {
            actor_id: alice_user(),
            session_id: None,
        };
        assert_eq!(actor_ref.on_behalf_of(), Some(alice_user()));
    }

    #[test]
    fn on_behalf_of_system_returns_on_behalf_of_actor() {
        let actor_ref = ActorRef::System {
            worker_name: "task-spawner".into(),
            on_behalf_of: Some(alice_user()),
        };
        assert_eq!(actor_ref.on_behalf_of(), Some(alice_user()));
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
                actor_id: alice_user(),
                session_id: None,
            })),
        };
        assert_eq!(actor_ref.on_behalf_of(), Some(alice_user()));
    }

    #[test]
    fn on_behalf_of_automation_recurses_through_nested_automations() {
        let inner = ActorRef::Automation {
            automation_name: "github_pr_sync".into(),
            triggered_by: Some(Box::new(ActorRef::Authenticated {
                actor_id: alice_user(),
                session_id: None,
            })),
        };
        let outer = ActorRef::Automation {
            automation_name: "link_conversation_to_artifacts".into(),
            triggered_by: Some(Box::new(inner)),
        };
        assert_eq!(outer.on_behalf_of(), Some(alice_user()));
    }

    #[test]
    fn on_behalf_of_automation_recurses_into_system_on_behalf_of() {
        let actor_ref = ActorRef::Automation {
            automation_name: "github_pr_sync".into(),
            triggered_by: Some(Box::new(ActorRef::System {
                worker_name: "task-spawner".into(),
                on_behalf_of: Some(adhoc_session()),
            })),
        };
        assert_eq!(actor_ref.on_behalf_of(), Some(adhoc_session()));
    }

    #[test]
    fn on_behalf_of_automation_without_trigger_returns_none() {
        let actor_ref = ActorRef::Automation {
            automation_name: "standalone".into(),
            triggered_by: None,
        };
        assert_eq!(actor_ref.on_behalf_of(), None);
    }

    // --- TryFrom<ActorIdentity> ---

    #[test]
    fn try_from_actor_identity_user() {
        let identity = ActorIdentity::User {
            username: Username::try_new("alice").unwrap(),
        };
        let actor_id = ActorId::try_from(identity).unwrap();
        assert_eq!(actor_id, alice_user());
    }

    #[test]
    fn try_from_actor_identity_agent() {
        let identity = ActorIdentity::Agent {
            name: AgentName::try_new("swe").unwrap(),
            creator: Username::try_new("creator").unwrap(),
        };
        let actor_id = ActorId::try_from(identity).unwrap();
        assert_eq!(actor_id, swe_agent());
    }

    #[test]
    fn try_from_actor_identity_adhoc() {
        let sid = SessionId::from_str("s-abcdef").unwrap();
        let identity = ActorIdentity::Adhoc {
            session_id: sid.clone(),
            creator: Username::try_new("creator").unwrap(),
        };
        let actor_id = ActorId::try_from(identity).unwrap();
        assert_eq!(actor_id, ActorId::Adhoc(sid));
    }
}
