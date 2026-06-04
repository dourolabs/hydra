//! Server-side domain types and validation for triggered actions.
//!
//! See `/designs/triggered-actions.md` §4.3 and §4.5.
//!
//! This module owns the template renderer, `Trigger::validate`, and the
//! `Action::run` dispatch — the one place per-action logic lives. The
//! `ScheduledTriggerWorker` calls `Action::run` for each due trigger.

use crate::domain::actors::ActorRef;
use crate::domain::issues::Issue as DomainIssue;
use crate::store::{RelationshipType, Store, StoreError};
use chrono::{DateTime, Utc};
use cron::Schedule as CronSchedule;
use hydra_common::HydraId;
use hydra_common::issues::IssueStatus;
use hydra_common::principal::Principal;
use hydra_common::triggers::{Action, CreateIssueAction, Schedule, Trigger};
use hydra_common::{IssueId, RepoName, TriggerId};
use std::str::FromStr;
use thiserror::Error;
use tracing::warn;

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

/// Failure modes produced by [`run_action`].
///
/// `Render` and `ParseAssignee` are deterministic config errors caught
/// per the §4.5 contract — the worker logs them and continues. `Store`
/// is a transient persistence failure; the worker still records the
/// fire (per §4.6) and moves on.
#[derive(Debug, Error)]
pub enum ActionError {
    #[error("template field '{field}' failed to render: {source}")]
    Render {
        field: &'static str,
        #[source]
        source: RenderError,
    },
    #[error("rendered assignee '{rendered}' did not parse as a Principal: {detail}")]
    ParseAssignee { rendered: String, detail: String },
    #[error("store operation failed: {source}")]
    Store {
        #[source]
        source: StoreError,
    },
}

/// Dispatch entry-point for one action of a firing trigger.
///
/// `store` is the raw `&dyn Store` so this module stays domain-level
/// (it does not depend on `StoreWithEvents` or the event bus). The
/// `ScheduledTriggerWorker` is the only production caller; unit tests
/// drive it directly against a `MemoryStore`.
///
/// Returns `ActionTarget::Issue(id)` on success. Errors are surfaced
/// to the worker which logs them and continues to the next action.
pub async fn run_action(
    action: &Action,
    ctx: &RenderContext,
    store: &dyn Store,
    actor: &ActorRef,
    trigger_id: &TriggerId,
) -> Result<ActionTarget, ActionError> {
    match action {
        Action::CreateIssue(create) => {
            run_create_issue(create, ctx, store, actor, trigger_id).await
        }
    }
}

async fn run_create_issue(
    action: &CreateIssueAction,
    ctx: &RenderContext,
    store: &dyn Store,
    actor: &ActorRef,
    trigger_id: &TriggerId,
) -> Result<ActionTarget, ActionError> {
    let title = render(&action.title, ctx).map_err(|source| ActionError::Render {
        field: "title",
        source,
    })?;
    let description = render(&action.description, ctx).map_err(|source| ActionError::Render {
        field: "description",
        source,
    })?;
    let assignee = match action.assignee.as_deref() {
        Some(template) => {
            let rendered = render(template, ctx).map_err(|source| ActionError::Render {
                field: "assignee",
                source,
            })?;
            let trimmed = rendered.trim();
            if trimmed.is_empty() {
                None
            } else {
                let parsed =
                    Principal::from_str(trimmed).map_err(|err| ActionError::ParseAssignee {
                        rendered: rendered.clone(),
                        detail: err.to_string(),
                    })?;
                Some(parsed)
            }
        }
        None => None,
    };

    // Resolve the trigger's creator from the `on_behalf_of` field. v1's
    // worker always sets this to `Some(ActorId::User(creator))`, so a
    // missing or non-user `on_behalf_of` is a worker contract bug — we
    // surface it as a Store error so the per-action log mentions the
    // trigger and the next action still gets a chance to run.
    let creator = match actor.on_behalf_of() {
        Some(hydra_common::ActorId::User(name)) => name,
        _ => {
            return Err(ActionError::Store {
                source: StoreError::Internal(format!(
                    "ActorRef::Trigger for {trigger_id} did not carry a User on_behalf_of",
                )),
            });
        }
    };

    let issue = DomainIssue::new(
        action.issue_type.into(),
        title,
        description,
        creator.into(),
        String::new(),
        action.status.unwrap_or(IssueStatus::Open).into(),
        assignee,
        Some(action.session_settings.clone().into()),
        Vec::new(),
        Vec::new(),
        None,
        None,
        None,
    );

    let (issue_id, _version) = store
        .add_issue(issue, actor)
        .await
        .map_err(|source| ActionError::Store { source })?;

    // Follow-up `created` edge — best-effort per §4.2. A failure here
    // leaves the issue intact (and still attributed to `ActorRef::Trigger`,
    // so audit lineage holds) and only loses one row of the firing-history
    // panel. The design accepts that trade-off.
    let source = HydraId::from(trigger_id.clone());
    let target = HydraId::from(issue_id.clone());
    if let Err(err) = store
        .add_relationship(&source, &target, RelationshipType::Created)
        .await
    {
        warn!(
            trigger_id = %trigger_id,
            issue_id = %issue_id,
            error = %err,
            "failed to write Trigger -created-> Issue edge; firing-history panel will miss one row"
        );
    }

    Ok(ActionTarget::Issue(issue_id))
}

/// Extension trait so callers can write `action.run(...)`. `Action`
/// lives in `hydra-common`, so the inherent method form (`impl Action {
/// pub async fn run(...) }`) is not available — the trait is the
/// orphan-rule workaround. Semantics match [`run_action`].
pub trait ActionRun {
    fn run<'a>(
        &'a self,
        ctx: &'a RenderContext,
        store: &'a dyn Store,
        actor: &'a ActorRef,
        trigger_id: &'a TriggerId,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ActionTarget, ActionError>> + Send + 'a>,
    >;
}

impl ActionRun for Action {
    fn run<'a>(
        &'a self,
        ctx: &'a RenderContext,
        store: &'a dyn Store,
        actor: &'a ActorRef,
        trigger_id: &'a TriggerId,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ActionTarget, ActionError>> + Send + 'a>,
    > {
        Box::pin(run_action(self, ctx, store, actor, trigger_id))
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

    // ---- Action::run --------------------------------------------------

    mod run_tests {
        use super::*;
        use crate::store::{MemoryStore, ReadOnlyStore, RelationshipType, Store};
        use hydra_common::ActorId;
        use hydra_common::api::v1::users::Username as ApiUsername;

        fn trigger_actor(trigger_id: TriggerId, creator: &str) -> ActorRef {
            ActorRef::Trigger {
                trigger_id,
                on_behalf_of: Some(ActorId::User(ApiUsername::from(creator))),
            }
        }

        async fn add_user(store: &MemoryStore, username: &str) {
            use crate::domain::users::{User, Username};
            store
                .add_user(
                    User::new(Username::from(username), None, false),
                    &ActorRef::test(),
                )
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn create_issue_renders_fields_and_writes_created_edge() {
            let store = MemoryStore::new();
            add_user(&store, "alice").await;
            let trigger_id = TriggerId::new();
            let actor = trigger_actor(trigger_id.clone(), "alice");
            let action = create_issue(
                "Daily {{ now.date }}",
                "Trigger {{ trigger.id }} fired at {{ scheduled_at }}",
                Some("users/alice"),
                None,
            );

            let ctx = RenderContext::new(
                "2026-06-03T15:04:05Z".parse().unwrap(),
                "2026-06-03T15:00:00Z".parse().unwrap(),
                trigger_id.clone(),
            );

            let target = run_action(&action, &ctx, &store, &actor, &trigger_id)
                .await
                .expect("action should succeed");
            let ActionTarget::Issue(issue_id) = target;

            // Issue persisted with rendered fields and the trigger actor.
            let versioned = store.get_issue(&issue_id, false).await.unwrap();
            assert_eq!(versioned.item.title, "Daily 2026-06-03");
            assert_eq!(
                versioned.item.description,
                format!("Trigger {trigger_id} fired at 2026-06-03T15:00:00+00:00"),
            );
            assert_eq!(versioned.item.creator.as_str(), "alice");
            assert!(matches!(versioned.actor, Some(ActorRef::Trigger { .. }),));

            // Exactly one created edge between the trigger and the issue.
            let source = HydraId::from(trigger_id.clone());
            let target = HydraId::from(issue_id.clone());
            let edges = store
                .get_relationships(
                    Some(&source),
                    Some(&target),
                    Some(RelationshipType::Created),
                )
                .await
                .unwrap();
            assert_eq!(edges.len(), 1);
            assert_eq!(edges[0].rel_type, RelationshipType::Created);
        }

        #[tokio::test]
        async fn create_issue_with_no_assignee_template_leaves_unassigned() {
            let store = MemoryStore::new();
            let trigger_id = TriggerId::new();
            let actor = trigger_actor(trigger_id.clone(), "alice");
            let action = create_issue("t", "d", None, None);

            let ctx = RenderContext::new(Utc::now(), Utc::now(), trigger_id.clone());
            let target = run_action(&action, &ctx, &store, &actor, &trigger_id)
                .await
                .unwrap();
            let ActionTarget::Issue(issue_id) = target;
            let issue = store.get_issue(&issue_id, false).await.unwrap();
            assert!(issue.item.assignee.is_none());
        }

        #[tokio::test]
        async fn missing_template_variable_returns_render_error() {
            let store = MemoryStore::new();
            let trigger_id = TriggerId::new();
            let actor = trigger_actor(trigger_id.clone(), "alice");
            // `{{ bogus }}` is not in `KNOWN_VARIABLES`. The renderer
            // surfaces `RenderError::UnknownVariable`; `Action::run`
            // wraps it in `ActionError::Render`.
            let action = create_issue("hi {{ bogus }}", "d", None, None);

            let ctx = RenderContext::new(Utc::now(), Utc::now(), trigger_id.clone());
            let err = run_action(&action, &ctx, &store, &actor, &trigger_id)
                .await
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    ActionError::Render {
                        field: "title",
                        source: RenderError::UnknownVariable { .. }
                    }
                ),
                "got {err:?}"
            );
            // No issue was created.
            let listed = store.list_issues(&Default::default()).await.unwrap();
            assert!(listed.is_empty(), "no issue should be created on failure");
        }

        #[tokio::test]
        async fn unparseable_assignee_returns_parse_error() {
            let store = MemoryStore::new();
            let trigger_id = TriggerId::new();
            let actor = trigger_actor(trigger_id.clone(), "alice");
            // `not-a-principal` is not in the `users/<x>` form.
            let action = create_issue("t", "d", Some("not-a-principal"), None);

            let ctx = RenderContext::new(Utc::now(), Utc::now(), trigger_id.clone());
            let err = run_action(&action, &ctx, &store, &actor, &trigger_id)
                .await
                .unwrap_err();
            assert!(
                matches!(err, ActionError::ParseAssignee { .. }),
                "got {err:?}"
            );
            let listed = store.list_issues(&Default::default()).await.unwrap();
            assert!(listed.is_empty());
        }
    }
}
