use super::issues::{IssueStatus, IssueType, SessionSettings};
use super::users::Username;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// API shape for a scheduled trigger.
///
/// See `/designs/triggered-actions.md` §4.3.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct Trigger {
    #[serde(default)]
    pub enabled: bool,
    pub schedule: Schedule,
    #[serde(default)]
    pub actions: Vec<Action>,
    pub creator: Username,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fired_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
}

impl Trigger {
    pub fn new(
        enabled: bool,
        schedule: Schedule,
        actions: Vec<Action>,
        creator: Username,
        last_fired_at: Option<DateTime<Utc>>,
        deleted: bool,
    ) -> Self {
        Self {
            enabled,
            schedule,
            actions,
            creator,
            last_fired_at,
            deleted,
        }
    }
}

/// When a trigger fires.
///
/// `Cron` is a 5-field cron expression. `Once { at }` fires a single time
/// at the given UTC timestamp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
// `Schedule` is a generic name; rename in TS to avoid collision with
// future unrelated `Schedule` exports.
#[cfg_attr(feature = "ts", ts(export, rename = "TriggerSchedule"))]
pub enum Schedule {
    Cron {
        expression: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timezone: Option<String>,
    },
    Once {
        at: DateTime<Utc>,
    },
}

/// One action in a trigger's `actions` list.
///
/// v1 ships only `CreateIssue`; future variants slot in without changing
/// surrounding machinery.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
// Rename in TS to avoid colliding with the form-module `Action` export.
#[cfg_attr(feature = "ts", ts(export, rename = "TriggerAction"))]
pub enum Action {
    CreateIssue(CreateIssueAction),
}

/// Create an issue when the parent trigger fires.
///
/// `title`, `description`, and `assignee` are template strings rendered
/// through `hydra-server/src/domain/triggers.rs::render`. `assignee` is
/// parsed as a `Principal` after rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct CreateIssueAction {
    #[serde(rename = "type")]
    pub issue_type: IssueType,
    pub title: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<IssueStatus>,
    #[serde(default, skip_serializing_if = "SessionSettings::is_default")]
    pub session_settings: SessionSettings,
}

impl CreateIssueAction {
    pub fn new(
        issue_type: IssueType,
        title: String,
        description: String,
        assignee: Option<String>,
        status: Option<IssueStatus>,
        session_settings: SessionSettings,
    ) -> Self {
        Self {
            issue_type,
            title,
            description,
            assignee,
            status,
            session_settings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_action() -> Action {
        Action::CreateIssue(CreateIssueAction::new(
            IssueType::Task,
            "Daily triage".to_string(),
            "Run triage for {{ now.date }}".to_string(),
            Some("users/alice".to_string()),
            Some(IssueStatus::Open),
            SessionSettings::default(),
        ))
    }

    #[test]
    fn trigger_round_trips_cron() {
        let trigger = Trigger::new(
            true,
            Schedule::Cron {
                expression: "0 9 * * 1".to_string(),
                timezone: Some("UTC".to_string()),
            },
            vec![sample_action()],
            Username::from("alice"),
            None,
            false,
        );
        let value = serde_json::to_value(&trigger).unwrap();
        let back: Trigger = serde_json::from_value(value).unwrap();
        assert_eq!(trigger, back);
    }

    #[test]
    fn trigger_round_trips_once() {
        let at: DateTime<Utc> = "2026-12-31T23:59:00Z".parse().unwrap();
        let trigger = Trigger::new(
            true,
            Schedule::Once { at },
            vec![sample_action()],
            Username::from("alice"),
            Some(at),
            false,
        );
        let value = serde_json::to_value(&trigger).unwrap();
        let back: Trigger = serde_json::from_value(value).unwrap();
        assert_eq!(trigger, back);
    }

    #[test]
    fn schedule_cron_wire_tag() {
        let s = Schedule::Cron {
            expression: "* * * * *".to_string(),
            timezone: None,
        };
        let value = serde_json::to_value(&s).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"Cron": {"expression": "* * * * *"}}),
        );
    }

    #[test]
    fn action_create_issue_wire_tag() {
        let a = sample_action();
        let value = serde_json::to_value(&a).unwrap();
        assert!(value.get("CreateIssue").is_some());
    }
}
