use crate::api::v1::users::Username;
use crate::ids::{IssueId, TaskId};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum ActorId {
    Username(Username),
    Task(TaskId),
    Issue(IssueId),
}

impl fmt::Display for ActorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActorId::Username(username) => write!(f, "u-{username}"),
            ActorId::Task(task_id) => write!(f, "w-{task_id}"),
            ActorId::Issue(issue_id) => write!(f, "a-{issue_id}"),
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
            ActorRef::Authenticated { actor_id } => match actor_id {
                ActorId::Username(username) => username.to_string(),
                ActorId::Task(task_id) => task_id.to_string(),
                ActorId::Issue(issue_id) => issue_id.to_string(),
            },
            ActorRef::System {
                worker_name,
                on_behalf_of,
            } => {
                if let Some(behalf) = on_behalf_of {
                    let behalf_name = match behalf {
                        ActorId::Username(username) => username.to_string(),
                        ActorId::Task(task_id) => task_id.to_string(),
                        ActorId::Issue(issue_id) => issue_id.to_string(),
                    };
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
}

/// Parse a user-facing shorthand string into an `ActorId`.
///
/// Shorthand rules:
/// - Strings starting with `"i-"` are parsed as [`IssueId`] → `ActorId::Issue`
/// - Strings starting with `"t-"` are parsed as [`TaskId`] → `ActorId::Task`
/// - Everything else is treated as a username → `ActorId::Username`
///
/// **Note:** This `FromStr` deliberately does NOT round-trip with [`Display`],
/// which uses the canonical prefixed format (`u-`, `a-`, `w-`). This `FromStr`
/// is for *user-facing CLI shorthand*, while `Display` is for the *wire/canonical
/// format*. Use [`parse_actor_name`] to parse the canonical format.
impl FromStr for ActorId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err("actor ID must not be empty".to_string());
        }

        if trimmed.starts_with("i-") {
            let issue_id = IssueId::from_str(trimmed)
                .map_err(|e| format!("invalid issue ID '{trimmed}': {e}"))?;
            return Ok(ActorId::Issue(issue_id));
        }

        if trimmed.starts_with("t-") {
            let task_id = TaskId::from_str(trimmed)
                .map_err(|e| format!("invalid task ID '{trimmed}': {e}"))?;
            return Ok(ActorId::Task(task_id));
        }

        Ok(ActorId::Username(Username::from(trimmed)))
    }
}

/// Parse an actor name string (e.g. `u-alice` or `w-t-abcdef`) into an `ActorId`.
///
/// Returns `None` if the name does not match a recognized prefix or is otherwise
/// invalid.
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
        let task_id = TaskId::from_str(task_id).ok()?;
        return Some(ActorId::Task(task_id));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_ref_serialization_round_trip_authenticated() {
        let actor_ref = ActorRef::Authenticated {
            actor_id: ActorId::Username(Username::from("bob")),
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
        let task_id = TaskId::from_str("t-abcdef").unwrap();
        let result = parse_actor_name("w-t-abcdef");
        assert_eq!(result, Some(ActorId::Task(task_id)));
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
        let task_id = TaskId::from_str("t-abcdef").unwrap();
        let actor_id = ActorId::Task(task_id);
        assert_eq!(actor_id.to_string(), "w-t-abcdef");
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
        let actor: ActorId = "t-abcdef".parse().unwrap();
        match actor {
            ActorId::Task(id) => assert_eq!(id.to_string(), "t-abcdef"),
            other => panic!("expected ActorId::Task, got {other:?}"),
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
}
