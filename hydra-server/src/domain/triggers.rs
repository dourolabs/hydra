//! Server-side domain types and validation for triggered actions.
//!
//! See `/designs/triggered-actions.md` §4.3 and §4.5.
//!
//! This module owns the template renderer and `Trigger::validate`. The
//! `impl Action { pub async fn run(...) }` body lands in PR 5 alongside
//! the worker that calls it.

use chrono::{DateTime, Utc};
use cron::Schedule as CronSchedule;
use hydra_common::triggers::{Action, CreateIssueAction, Schedule, Trigger};
use hydra_common::{IssueId, RepoName, TriggerId};
use std::str::FromStr;
use thiserror::Error;

/// Object produced by a successful `Action::run`.
///
/// v1 only ships `Issue`; future action variants (`CreateConversation`,
/// …) drop in as additional arms here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionTarget {
    Issue(IssueId),
}

/// Variables available to template strings.
///
/// See §4.5: `now.iso`, `now.date`, `scheduled_at`, `trigger.id`.
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
const KNOWN_VARIABLES: &[&str] = &["now.iso", "now.date", "scheduled_at", "trigger.id"];

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

/// Parse a 5-field cron expression (the design's wire format) into a
/// [`cron::Schedule`]. The `cron` crate expects 6 fields (with seconds);
/// we prepend `0 ` so user-typed `m h dom mon dow` parses correctly.
///
/// Returns the cron crate's error message on failure; the caller wraps
/// it in [`ValidationError::InvalidCron`] with the original expression.
pub fn parse_cron_expression(expression: &str) -> Result<CronSchedule, String> {
    let normalised = format!("0 {}", expression.trim());
    CronSchedule::from_str(&normalised).map_err(|e| e.to_string())
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
/// names, but skip substitution. Used by `Trigger::validate` so callers
/// can lint a stored trigger without supplying a `RenderContext`.
fn validate_template(template: &str) -> Result<(), RenderError> {
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
            // Disallow a stray '{{' inside the variable region — it would
            // otherwise let `{{ foo {{ bar }}` parse as `foo {{ bar` and
            // mask the real unbalanced-open error.
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
                // Lookup is infallible here because we just verified `raw`
                // is in `KNOWN_VARIABLES`; the `expect` is a defense
                // against the two lists drifting apart.
                let value = c
                    .lookup(raw)
                    .expect("KNOWN_VARIABLES and RenderContext::lookup must agree");
                out.push_str(&value);
            }
            i = var_end + 2;
        } else if rest.starts_with("}}") {
            return Err(RenderError::UnbalancedClose { position: i });
        } else {
            // Take one char to handle multibyte boundaries cleanly.
            let ch = rest.chars().next().expect("rest is non-empty");
            if ctx.is_some() {
                out.push(ch);
            }
            i += ch.len_utf8();
        }
    }
    Ok(out)
}

/// A non-fatal issue raised during [`Trigger::validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationWarning {
    /// A `Schedule::Once { at }` whose `at` is already in the past at
    /// validate time. The trigger will not fire but is otherwise valid.
    PastOnce { at: DateTime<Utc> },
}

/// Failure modes produced by [`Trigger::validate`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("invalid cron expression '{expression}': {detail}")]
    InvalidCron { expression: String, detail: String },
    #[error("action {action_index}: template field '{field}': {source}")]
    InvalidTemplate {
        action_index: usize,
        field: &'static str,
        source: RenderError,
    },
    #[error(
        "action {action_index}: session_settings.repo_name '{repo_name}' is not a known repository"
    )]
    UnknownRepoName {
        action_index: usize,
        repo_name: RepoName,
    },
    #[error("action {action_index}: unsupported issue type")]
    UnknownIssueType { action_index: usize },
    #[error("action {action_index}: unsupported issue status")]
    UnknownIssueStatus { action_index: usize },
}

/// Validate a [`Trigger`]'s schedule, actions, and template fields.
///
/// Returns the list of non-fatal warnings on success. The first
/// structural failure short-circuits with `Err`.
///
/// `known_repos` is the catalog of valid `RepoName`s supplied by the HTTP
/// route layer (PR 5) from the store; callers may pass an empty slice
/// when no `repo_name` is set on any action.
pub fn validate(
    trigger: &Trigger,
    known_repos: &[RepoName],
) -> Result<Vec<ValidationWarning>, ValidationError> {
    let mut warnings = Vec::new();

    match &trigger.schedule {
        Schedule::Cron { expression, .. } => {
            parse_cron_expression(expression).map_err(|detail| ValidationError::InvalidCron {
                expression: expression.clone(),
                detail,
            })?;
        }
        Schedule::Once { at } => {
            if *at < Utc::now() {
                warnings.push(ValidationWarning::PastOnce { at: *at });
            }
        }
    }

    for (idx, action) in trigger.actions.iter().enumerate() {
        match action {
            Action::CreateIssue(create) => {
                validate_create_issue(idx, create, known_repos)?;
            }
        }
    }

    Ok(warnings)
}

fn validate_create_issue(
    action_index: usize,
    action: &CreateIssueAction,
    known_repos: &[RepoName],
) -> Result<(), ValidationError> {
    use hydra_common::issues::{IssueStatus, IssueType};

    if matches!(action.issue_type, IssueType::Unknown) {
        return Err(ValidationError::UnknownIssueType { action_index });
    }
    if matches!(action.status, Some(IssueStatus::Unknown)) {
        return Err(ValidationError::UnknownIssueStatus { action_index });
    }

    for (field, value) in [
        ("title", &action.title),
        ("description", &action.description),
    ] {
        validate_template(value).map_err(|source| ValidationError::InvalidTemplate {
            action_index,
            field,
            source,
        })?;
    }
    if let Some(assignee) = &action.assignee {
        validate_template(assignee).map_err(|source| ValidationError::InvalidTemplate {
            action_index,
            field: "assignee",
            source,
        })?;
    }

    if let Some(repo) = &action.session_settings.repo_name {
        if !known_repos.contains(repo) {
            return Err(ValidationError::UnknownRepoName {
                action_index,
                repo_name: repo.clone(),
            });
        }
    }

    Ok(())
}

/// Public extension trait so `Trigger::validate(&self, &known_repos)`
/// reads naturally at the call site, matching §4.3.
pub trait TriggerValidation {
    fn validate(&self, known_repos: &[RepoName])
    -> Result<Vec<ValidationWarning>, ValidationError>;
}

impl TriggerValidation for Trigger {
    fn validate(
        &self,
        known_repos: &[RepoName],
    ) -> Result<Vec<ValidationWarning>, ValidationError> {
        validate(self, known_repos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hydra_common::api::v1::issues::SessionSettings;
    use hydra_common::issues::{IssueStatus, IssueType};
    use hydra_common::users::Username;
    use std::str::FromStr;

    fn ctx() -> RenderContext {
        RenderContext::new(
            "2026-06-03T15:04:05Z".parse().unwrap(),
            "2026-06-03T15:00:00Z".parse().unwrap(),
            TriggerId::from_str("t-abcdef").unwrap(),
        )
    }

    // ---- renderer -------------------------------------------------------

    #[test]
    fn render_no_op_string() {
        assert_eq!(render("hello world", &ctx()).unwrap(), "hello world");
    }

    #[test]
    fn render_empty_string() {
        assert_eq!(render("", &ctx()).unwrap(), "");
    }

    #[test]
    fn render_single_variable() {
        assert_eq!(
            render("today: {{ now.date }}", &ctx()).unwrap(),
            "today: 2026-06-03"
        );
    }

    #[test]
    fn render_variable_without_whitespace() {
        assert_eq!(render("id:{{trigger.id}}", &ctx()).unwrap(), "id:t-abcdef");
    }

    #[test]
    fn render_multiple_substitutions() {
        let s = render(
            "trigger {{ trigger.id }} fired at {{ scheduled_at }} (date: {{ now.date }})",
            &ctx(),
        )
        .unwrap();
        assert_eq!(
            s,
            "trigger t-abcdef fired at 2026-06-03T15:00:00+00:00 (date: 2026-06-03)"
        );
    }

    #[test]
    fn render_iso_variable() {
        // Must produce a chrono-parseable RFC3339 string.
        let s = render("{{ now.iso }}", &ctx()).unwrap();
        let parsed: DateTime<Utc> = s.parse().unwrap();
        assert_eq!(parsed, ctx().now);
    }

    #[test]
    fn render_unbalanced_open_brace() {
        let err = render("hello {{ now.date", &ctx()).unwrap_err();
        assert!(
            matches!(err, RenderError::UnbalancedOpen { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn render_unbalanced_close_brace() {
        let err = render("hello }} world", &ctx()).unwrap_err();
        assert!(
            matches!(err, RenderError::UnbalancedClose { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn render_unknown_variable() {
        let err = render("{{ unknown }}", &ctx()).unwrap_err();
        assert!(
            matches!(err, RenderError::UnknownVariable { ref name } if name == "unknown"),
            "got {err:?}"
        );
    }

    #[test]
    fn render_empty_variable() {
        let err = render("{{  }}", &ctx()).unwrap_err();
        assert!(matches!(err, RenderError::EmptyVariable), "got {err:?}");
    }

    #[test]
    fn render_nested_open_inside_variable_fails() {
        let err = render("{{ now.date {{ trigger.id }} }}", &ctx()).unwrap_err();
        assert!(
            matches!(err, RenderError::UnbalancedOpen { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn render_lone_braces_pass_through() {
        // Single braces are not template syntax.
        assert_eq!(render("a { b } c", &ctx()).unwrap(), "a { b } c");
    }

    // ---- validate -------------------------------------------------------

    fn sample_repo() -> RepoName {
        RepoName::from_str("dourolabs/hydra").unwrap()
    }

    fn trigger_with_actions(schedule: Schedule, actions: Vec<Action>) -> Trigger {
        Trigger::new(
            true,
            schedule,
            actions,
            Username::from("alice"),
            None,
            false,
        )
    }

    fn create_issue(
        title: &str,
        description: &str,
        assignee: Option<&str>,
        repo: Option<RepoName>,
    ) -> Action {
        let mut settings = SessionSettings::default();
        settings.repo_name = repo;
        Action::CreateIssue(CreateIssueAction::new(
            IssueType::Task,
            title.to_string(),
            description.to_string(),
            assignee.map(str::to_string),
            Some(IssueStatus::Open),
            settings,
        ))
    }

    #[test]
    fn validate_accepts_well_formed_cron_trigger() {
        let trigger = trigger_with_actions(
            Schedule::Cron {
                expression: "0 9 * * MON".to_string(),
                timezone: Some("UTC".to_string()),
            },
            vec![create_issue(
                "Daily triage",
                "go {{ now.date }}",
                None,
                None,
            )],
        );
        assert_eq!(trigger.validate(&[]).unwrap(), vec![]);
    }

    #[test]
    fn cron_crate_format_probe() {
        // Document what the `cron` crate accepts so a future bump that
        // tightens the parser is caught here, not in production.
        // 5-field (the design's wire format) is rejected; we normalise to
        // 6-field by prepending "0 " (seconds) before parsing.
        assert!(CronSchedule::from_str("0 9 * * 1").is_err());
        assert!(CronSchedule::from_str("0 0 9 * * MON").is_ok());
    }

    #[test]
    fn validate_accepts_five_field_cron() {
        // The user types 5 fields per §4.3; validate must accept that.
        let trigger = trigger_with_actions(
            Schedule::Cron {
                expression: "0 9 * * MON".to_string(),
                timezone: Some("UTC".to_string()),
            },
            vec![],
        );
        assert!(trigger.validate(&[]).unwrap().is_empty());
    }

    #[test]
    fn validate_rejects_invalid_cron() {
        let trigger = trigger_with_actions(
            Schedule::Cron {
                expression: "not a cron".to_string(),
                timezone: None,
            },
            vec![],
        );
        let err = trigger.validate(&[]).unwrap_err();
        assert!(
            matches!(err, ValidationError::InvalidCron { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn validate_warns_on_past_once() {
        let at: DateTime<Utc> = "2020-01-01T00:00:00Z".parse().unwrap();
        let trigger = trigger_with_actions(Schedule::Once { at }, vec![]);
        let warnings = trigger.validate(&[]).unwrap();
        assert_eq!(warnings, vec![ValidationWarning::PastOnce { at }]);
    }

    #[test]
    fn validate_no_warnings_for_future_once() {
        let at = Utc::now() + chrono::Duration::hours(1);
        let trigger = trigger_with_actions(Schedule::Once { at }, vec![]);
        assert!(trigger.validate(&[]).unwrap().is_empty());
    }

    #[test]
    fn validate_rejects_unknown_template_variable() {
        let trigger = trigger_with_actions(
            Schedule::Cron {
                expression: "* * * * *".to_string(),
                timezone: None,
            },
            vec![create_issue("hi {{ bogus }}", "ok", None, None)],
        );
        let err = trigger.validate(&[]).unwrap_err();
        match err {
            ValidationError::InvalidTemplate {
                action_index,
                field,
                source: RenderError::UnknownVariable { name },
            } => {
                assert_eq!(action_index, 0);
                assert_eq!(field, "title");
                assert_eq!(name, "bogus");
            }
            other => panic!("unexpected err: {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_unbalanced_braces_in_description() {
        let trigger = trigger_with_actions(
            Schedule::Cron {
                expression: "* * * * *".to_string(),
                timezone: None,
            },
            vec![create_issue("title", "{{ now.date", None, None)],
        );
        let err = trigger.validate(&[]).unwrap_err();
        assert!(
            matches!(
                err,
                ValidationError::InvalidTemplate {
                    field: "description",
                    source: RenderError::UnbalancedOpen { .. },
                    ..
                }
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn validate_accepts_template_in_assignee() {
        let trigger = trigger_with_actions(
            Schedule::Cron {
                expression: "* * * * *".to_string(),
                timezone: None,
            },
            vec![create_issue(
                "title",
                "desc",
                Some("users/{{ trigger.id }}"),
                None,
            )],
        );
        assert!(trigger.validate(&[]).unwrap().is_empty());
    }

    #[test]
    fn validate_rejects_unknown_repo_name() {
        let known = sample_repo();
        let other = RepoName::from_str("acme/unknown").unwrap();
        let trigger = trigger_with_actions(
            Schedule::Cron {
                expression: "* * * * *".to_string(),
                timezone: None,
            },
            vec![create_issue("title", "desc", None, Some(other.clone()))],
        );
        let err = trigger.validate(std::slice::from_ref(&known)).unwrap_err();
        match err {
            ValidationError::UnknownRepoName {
                action_index,
                repo_name,
            } => {
                assert_eq!(action_index, 0);
                assert_eq!(repo_name, other);
            }
            other => panic!("unexpected err: {other:?}"),
        }
    }

    #[test]
    fn validate_accepts_known_repo_name() {
        let known = sample_repo();
        let trigger = trigger_with_actions(
            Schedule::Cron {
                expression: "* * * * *".to_string(),
                timezone: None,
            },
            vec![create_issue("title", "desc", None, Some(known.clone()))],
        );
        assert!(
            trigger
                .validate(std::slice::from_ref(&known))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn validate_rejects_unknown_issue_type() {
        let mut action = create_issue("title", "desc", None, None);
        let Action::CreateIssue(ref mut a) = action;
        a.issue_type = IssueType::Unknown;
        let trigger = trigger_with_actions(
            Schedule::Cron {
                expression: "* * * * *".to_string(),
                timezone: None,
            },
            vec![action],
        );
        let err = trigger.validate(&[]).unwrap_err();
        assert!(
            matches!(err, ValidationError::UnknownIssueType { action_index: 0 }),
            "got {err:?}"
        );
    }

    #[test]
    fn validate_rejects_unknown_issue_status() {
        let mut action = create_issue("title", "desc", None, None);
        let Action::CreateIssue(ref mut a) = action;
        a.status = Some(IssueStatus::Unknown);
        let trigger = trigger_with_actions(
            Schedule::Cron {
                expression: "* * * * *".to_string(),
                timezone: None,
            },
            vec![action],
        );
        let err = trigger.validate(&[]).unwrap_err();
        assert!(
            matches!(err, ValidationError::UnknownIssueStatus { action_index: 0 }),
            "got {err:?}"
        );
    }
}
