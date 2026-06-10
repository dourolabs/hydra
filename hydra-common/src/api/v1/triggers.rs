use super::issues::{IssueType, SessionSettings};
use super::projects::StatusKey;
use super::users::Username;
use crate::actor_ref::ActorRef;
use crate::ids::{ProjectId, TriggerId};
use crate::versioning::VersionNumber;
use chrono::{DateTime, Utc};
use cron::Schedule as CronSchedule;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

/// API shape for a scheduled trigger: a [`Schedule`] (cron or one-shot)
/// plus an ordered list of [`Action`]s to run on each fire. `creator`
/// owns the trigger; `last_fired_at` is the persisted slot the scheduler
/// already serviced, updated in-place after each tick so a restart never
/// double-fires the same slot and never replays slots whose scheduled
/// time elapsed during downtime.
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
/// through [`render`]. `assignee` is parsed as a `Principal` after
/// rendering. Both `project_id` and `status` are required on the wire —
/// no defaults, no inference. A persisted trigger row missing either
/// field will fail loudly at deserialization rather than silently
/// substitute a default.
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
    pub project_id: ProjectId,
    pub status: StatusKey,
    #[serde(default, skip_serializing_if = "SessionSettings::is_default")]
    pub session_settings: SessionSettings,
}

impl CreateIssueAction {
    pub fn new(
        issue_type: IssueType,
        title: String,
        description: String,
        assignee: Option<String>,
        project_id: ProjectId,
        status: StatusKey,
        session_settings: SessionSettings,
    ) -> Self {
        Self {
            issue_type,
            title,
            description,
            assignee,
            project_id,
            status,
            session_settings,
        }
    }
}

/// Body for `POST /v1/triggers` and `PUT /v1/triggers/:id`.
///
/// `last_fired_at` and `deleted` are stripped — they are owned by the
/// server (`last_fired_at` is written in-place by `record_trigger_fire`
/// and carried forward by `update_trigger`; `deleted` is flipped by
/// `delete_trigger`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpsertTriggerRequest {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub schedule: Schedule,
    #[serde(default)]
    pub actions: Vec<Action>,
    pub creator: Username,
}

fn default_enabled() -> bool {
    true
}

impl UpsertTriggerRequest {
    pub fn new(enabled: bool, schedule: Schedule, actions: Vec<Action>, creator: Username) -> Self {
        Self {
            enabled,
            schedule,
            actions,
            creator,
        }
    }
}

/// `POST /v1/triggers` and `PUT /v1/triggers/:id` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UpsertTriggerResponse {
    pub trigger_id: TriggerId,
    pub version: VersionNumber,
}

impl UpsertTriggerResponse {
    pub fn new(trigger_id: TriggerId, version: VersionNumber) -> Self {
        Self {
            trigger_id,
            version,
        }
    }
}

/// One version row for `GET /v1/triggers/:id`,
/// `GET /v1/triggers/:id/versions/:n`, and entries in
/// `ListTriggerVersionsResponse`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct TriggerVersionRecord {
    pub trigger_id: TriggerId,
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    pub trigger: Trigger,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ActorRef>,
    pub creation_time: DateTime<Utc>,
}

impl TriggerVersionRecord {
    pub fn new(
        trigger_id: TriggerId,
        version: VersionNumber,
        timestamp: DateTime<Utc>,
        trigger: Trigger,
        actor: Option<ActorRef>,
        creation_time: DateTime<Utc>,
    ) -> Self {
        Self {
            trigger_id,
            version,
            timestamp,
            trigger,
            actor,
            creation_time,
        }
    }
}

/// `GET /v1/triggers` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListTriggersResponse {
    pub triggers: Vec<TriggerVersionRecord>,
}

impl ListTriggersResponse {
    pub fn new(triggers: Vec<TriggerVersionRecord>) -> Self {
        Self { triggers }
    }
}

/// `GET /v1/triggers/:id/versions` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListTriggerVersionsResponse {
    pub versions: Vec<TriggerVersionRecord>,
}

impl ListTriggerVersionsResponse {
    pub fn new(versions: Vec<TriggerVersionRecord>) -> Self {
        Self { versions }
    }
}

/// `GET /v1/triggers` query string.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SearchTriggersQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_deleted: Option<bool>,
}

/// Variables available to template strings: `now.iso`, `now.date`,
/// `scheduled_at`, `trigger.id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderContext {
    pub now: DateTime<Utc>,
    pub scheduled_at: DateTime<Utc>,
    pub trigger_id: TriggerId,
}

impl RenderContext {
    pub fn new(now: DateTime<Utc>, scheduled_at: DateTime<Utc>, trigger_id: TriggerId) -> Self {
        Self {
            now,
            scheduled_at,
            trigger_id,
        }
    }

    fn lookup(&self, name: &str) -> Option<String> {
        match name {
            "now.iso" => Some(self.now.to_rfc3339()),
            "now.date" => Some(self.now.format("%Y-%m-%d").to_string()),
            "scheduled_at" => Some(self.scheduled_at.to_rfc3339()),
            "trigger.id" => Some(self.trigger_id.to_string()),
            _ => None,
        }
    }
}

/// All recognized template variables; the parse-only path consults this
/// set so `Trigger::validate` can reject an unknown variable without
/// requiring a fully populated `RenderContext`.
pub const KNOWN_VARIABLES: &[&str] = &["now.iso", "now.date", "scheduled_at", "trigger.id"];

/// Failure modes produced by [`render`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RenderError {
    #[error("unbalanced '{{{{' at byte {position}")]
    UnbalancedOpen { position: usize },
    #[error("unbalanced '}}}}' at byte {position}")]
    UnbalancedClose { position: usize },
    #[error("unknown template variable '{name}'")]
    UnknownVariable { name: String },
    #[error("empty template variable")]
    EmptyVariable,
}

/// Render `template` against `ctx`, substituting `{{ var }}` placeholders.
///
/// Whitespace around the variable name inside `{{ }}` is ignored.
/// Unknown variables, unbalanced braces, and empty `{{ }}` placeholders
/// produce [`RenderError`].
pub fn render(template: &str, ctx: &RenderContext) -> Result<String, RenderError> {
    parse_template(template, Some(ctx))
}

/// Parse-only variant: walk the template, validate braces and variable
/// names, but skip substitution. Used by callers (e.g. trigger validation)
/// that lint a stored template without having a `RenderContext`.
pub fn validate_template(template: &str) -> Result<(), RenderError> {
    parse_template(template, None).map(drop)
}

fn parse_template(template: &str, ctx: Option<&RenderContext>) -> Result<String, RenderError> {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &template[i..];
        if let Some(after_open) = rest.strip_prefix("{{") {
            let var_start = i + 2;
            let close_rel = after_open
                .find("}}")
                .ok_or(RenderError::UnbalancedOpen { position: i })?;
            let var_end = var_start + close_rel;
            if template[var_start..var_end].contains("{{") {
                return Err(RenderError::UnbalancedOpen { position: i });
            }
            let raw = template[var_start..var_end].trim();
            if raw.is_empty() {
                return Err(RenderError::EmptyVariable);
            }
            if !KNOWN_VARIABLES.contains(&raw) {
                return Err(RenderError::UnknownVariable {
                    name: raw.to_string(),
                });
            }
            if let Some(c) = ctx {
                let value = c
                    .lookup(raw)
                    .expect("KNOWN_VARIABLES and RenderContext::lookup must agree");
                out.push_str(&value);
            }
            i = var_end + 2;
        } else if rest.starts_with("}}") {
            return Err(RenderError::UnbalancedClose { position: i });
        } else {
            let ch = rest.chars().next().expect("rest is non-empty");
            if ctx.is_some() {
                out.push(ch);
            }
            i += ch.len_utf8();
        }
    }
    Ok(out)
}

/// Parse a 5-field cron expression (the design's wire format) into a
/// [`cron::Schedule`]. The `cron` crate expects 6 fields (with seconds);
/// we prepend `0 ` so user-typed `m h dom mon dow` parses correctly.
///
/// Returns the cron crate's error message on failure.
pub fn parse_cron_expression(expression: &str) -> Result<CronSchedule, String> {
    let normalised = format!("0 {}", expression.trim());
    CronSchedule::from_str(&normalised).map_err(|e| e.to_string())
}

/// Extension trait so a [`Schedule`] can answer "is this trigger due to
/// fire right now, and if so, at which slot?" in one constant-time call.
pub trait ScheduleFiring {
    /// Returns the slot the trigger should fire at, or `None` if it is
    /// not due.
    ///
    /// For [`Schedule::Cron`]: the most recent slot ≤ `now` that is strictly
    /// after `last_fire`.
    ///
    /// For [`Schedule::Once`]: returns `Some(at)` iff `last_fire.is_none()`
    /// and `at <= now`.
    fn get_fire_candidate(
        &self,
        last_fire: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>>;

    /// Returns the next slot strictly after `now` for which the trigger
    /// is due to fire — independent of `last_fire`. Useful for previewing
    /// "next fire" in client UIs without having to know whether the
    /// trigger has just been fired.
    fn next_fire_after(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>>;
}

impl ScheduleFiring for Schedule {
    fn get_fire_candidate(
        &self,
        last_fire: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        match self {
            Schedule::Cron { expression, .. } => {
                let schedule = parse_cron_expression(expression).ok()?;
                let candidate = if schedule.includes(now) {
                    Some(now)
                } else {
                    schedule.after(&now).next_back()
                };
                let candidate = candidate?;
                match last_fire {
                    Some(prev) if candidate <= prev => None,
                    _ => Some(candidate),
                }
            }
            Schedule::Once { at } => {
                if last_fire.is_some() || *at > now {
                    None
                } else {
                    Some(*at)
                }
            }
        }
    }

    fn next_fire_after(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Schedule::Cron { expression, .. } => {
                let schedule = parse_cron_expression(expression).ok()?;
                schedule.after(&now).next()
            }
            Schedule::Once { at } => (*at > now).then_some(*at),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::status::status;

    fn sample_action() -> Action {
        Action::CreateIssue(CreateIssueAction::new(
            IssueType::Task,
            "Daily triage".to_string(),
            "Run triage for {{ now.date }}".to_string(),
            Some("users/alice".to_string()),
            ProjectId::try_from("j-defaul".to_string()).expect("well-formed ProjectId"),
            status("open"),
            SessionSettings::default(),
        ))
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
