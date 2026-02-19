use crate::api::v1::users::Username;
use crate::ids::TaskId;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum ActorId {
    Username(Username),
    Task(TaskId),
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
            },
            ActorRef::System {
                worker_name,
                on_behalf_of,
            } => {
                if let Some(behalf) = on_behalf_of {
                    let behalf_name = match behalf {
                        ActorId::Username(username) => username.to_string(),
                        ActorId::Task(task_id) => task_id.to_string(),
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
}
