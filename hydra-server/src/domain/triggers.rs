//! Server-side domain types and validation for triggered actions.
//!
//! This module owns the template renderer, `Trigger::validate`, and the
//! `Action::run` dispatch — the one place per-action logic lives. The
//! `ScheduledTriggerWorker` calls `Action::run` for each due trigger.

use crate::app::StoreWithEvents;
use crate::domain::actors::ActorRef;
use crate::domain::issues::Issue as DomainIssue;
use crate::store::{RelationshipType, StoreError};
use chrono::{DateTime, Utc};
use hydra_common::HydraId;
use hydra_common::api::v1::users::Username as ApiUsername;
use hydra_common::issues::IssueStatus;
use hydra_common::principal::Principal;
use hydra_common::triggers::{
    Action, CreateIssueAction, Schedule, Trigger, parse_cron_expression, render, validate_template,
};

// Re-export so existing call sites continue to work via
// `crate::domain::triggers::{RenderContext, RenderError, ScheduleFiring}`.
pub use hydra_common::triggers::{RenderContext, RenderError, ScheduleFiring};
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

/// A non-fatal issue raised during [`Trigger::validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationWarning {
    /// A `Schedule::Once { at }` whose `at` is already in the past at
    /// validate time. The trigger will not fire but is otherwise valid.
    PastOnce { at: DateTime<Utc> },
}

/// Failure modes produced by [`Trigger::validate`].
///
/// Self-consistency checks only — repo existence is verified separately
/// at the route/application boundary (`AppState`), so a stored `Trigger`
/// can be linted without a store handle.
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
/// Only pure self-consistency of the trigger payload is checked here.
/// External references (e.g. `session_settings.repo_name` existing in
/// the repository catalog) are validated by the application layer
/// against the store.
pub fn validate(trigger: &Trigger) -> Result<Vec<ValidationWarning>, ValidationError> {
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
                validate_create_issue(idx, create)?;
            }
        }
    }

    Ok(warnings)
}

fn validate_create_issue(
    action_index: usize,
    action: &CreateIssueAction,
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

    Ok(())
}

/// Returns the list of repo names referenced by `trigger`'s actions
/// (deduplicated, order-preserving). The application layer uses this to
/// drive targeted `Store::get_repository` lookups; the trigger payload
/// itself is validated by [`validate`] without a store handle.
pub fn referenced_repos(trigger: &Trigger) -> Vec<RepoName> {
    let mut out: Vec<RepoName> = Vec::new();
    for action in &trigger.actions {
        match action {
            Action::CreateIssue(create) => {
                if let Some(repo) = &create.session_settings.repo_name {
                    if !out.contains(repo) {
                        out.push(repo.clone());
                    }
                }
            }
        }
    }
    out
}

/// Public extension trait so `Trigger::validate()` reads naturally at
/// the call site.
pub trait TriggerValidation {
    fn validate(&self) -> Result<Vec<ValidationWarning>, ValidationError>;
}

impl TriggerValidation for Trigger {
    fn validate(&self) -> Result<Vec<ValidationWarning>, ValidationError> {
        validate(self)
    }
}

/// Failure modes produced by [`run_action`].
///
/// `Render` and `ParseAssignee` are deterministic config errors — the
/// worker logs them and continues. `Store` is a transient persistence
/// failure; the worker still records the fire and moves on.
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
/// Writes go through `StoreWithEvents` so trigger-created issues land on
/// the same event bus as any other write — automations elsewhere can
/// react to them. `actor` carries the firing trigger's identity for
/// audit; `creator` is copied from the trigger's own `creator` field
/// and used as the issue's `creator` (the two are conceptually
/// distinct).
///
/// Returns `ActionTarget::Issue(id)` on success. Errors are surfaced
/// to the worker which logs them and continues to the next action.
pub async fn run_action(
    action: &Action,
    ctx: &RenderContext,
    store: &StoreWithEvents,
    actor: &ActorRef,
    creator: &ApiUsername,
    trigger_id: &TriggerId,
) -> Result<ActionTarget, ActionError> {
    match action {
        Action::CreateIssue(create) => {
            run_create_issue(create, ctx, store, actor, creator, trigger_id).await
        }
    }
}

async fn run_create_issue(
    action: &CreateIssueAction,
    ctx: &RenderContext,
    store: &StoreWithEvents,
    actor: &ActorRef,
    creator: &ApiUsername,
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

    let issue = DomainIssue::new(
        action.issue_type.into(),
        title,
        description,
        creator.clone().into(),
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
        .add_issue_with_actor(issue, actor.clone())
        .await
        .map_err(|source| ActionError::Store { source })?;

    // Best-effort `created` edge: a failure here leaves the issue intact
    // (still attributed to `ActorRef::Trigger`, so audit lineage holds)
    // and only loses one row of the firing-history panel.
    let source = HydraId::from(trigger_id.clone());
    let target = HydraId::from(issue_id.clone());
    if let Err(err) = store
        .add_relationship_with_actor(&source, &target, RelationshipType::Created, actor.clone())
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
        store: &'a StoreWithEvents,
        actor: &'a ActorRef,
        creator: &'a ApiUsername,
        trigger_id: &'a TriggerId,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ActionTarget, ActionError>> + Send + 'a>,
    >;
}

impl ActionRun for Action {
    fn run<'a>(
        &'a self,
        ctx: &'a RenderContext,
        store: &'a StoreWithEvents,
        actor: &'a ActorRef,
        creator: &'a ApiUsername,
        trigger_id: &'a TriggerId,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ActionTarget, ActionError>> + Send + 'a>,
    > {
        Box::pin(run_action(self, ctx, store, actor, creator, trigger_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cron::Schedule as CronSchedule;
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
        assert_eq!(trigger.validate().unwrap(), vec![]);
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
        // The user types 5 fields per the spec; validate must accept that.
        let trigger = trigger_with_actions(
            Schedule::Cron {
                expression: "0 9 * * MON".to_string(),
                timezone: Some("UTC".to_string()),
            },
            vec![],
        );
        assert!(trigger.validate().unwrap().is_empty());
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
        let err = trigger.validate().unwrap_err();
        assert!(
            matches!(err, ValidationError::InvalidCron { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn validate_warns_on_past_once() {
        let at: DateTime<Utc> = "2020-01-01T00:00:00Z".parse().unwrap();
        let trigger = trigger_with_actions(Schedule::Once { at }, vec![]);
        let warnings = trigger.validate().unwrap();
        assert_eq!(warnings, vec![ValidationWarning::PastOnce { at }]);
    }

    #[test]
    fn validate_no_warnings_for_future_once() {
        let at = Utc::now() + chrono::Duration::hours(1);
        let trigger = trigger_with_actions(Schedule::Once { at }, vec![]);
        assert!(trigger.validate().unwrap().is_empty());
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
        let err = trigger.validate().unwrap_err();
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
        let err = trigger.validate().unwrap_err();
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
        assert!(trigger.validate().unwrap().is_empty());
    }

    #[test]
    fn referenced_repos_deduplicates_in_order() {
        let a = sample_repo();
        let b = RepoName::from_str("acme/other").unwrap();
        let trigger = trigger_with_actions(
            Schedule::Cron {
                expression: "* * * * *".to_string(),
                timezone: None,
            },
            vec![
                create_issue("t", "d", None, Some(a.clone())),
                create_issue("t", "d", None, Some(b.clone())),
                create_issue("t", "d", None, Some(a.clone())),
                create_issue("t", "d", None, None),
            ],
        );
        assert_eq!(referenced_repos(&trigger), vec![a, b]);
    }

    #[test]
    fn validate_ignores_repo_name_existence() {
        // Repo existence is now an application-layer concern; `validate`
        // accepts any `repo_name` regardless of whether it resolves.
        let trigger = trigger_with_actions(
            Schedule::Cron {
                expression: "* * * * *".to_string(),
                timezone: None,
            },
            vec![create_issue(
                "title",
                "desc",
                None,
                Some(RepoName::from_str("acme/unknown").unwrap()),
            )],
        );
        assert!(trigger.validate().unwrap().is_empty());
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
        let err = trigger.validate().unwrap_err();
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
        let err = trigger.validate().unwrap_err();
        assert!(
            matches!(err, ValidationError::UnknownIssueStatus { action_index: 0 }),
            "got {err:?}"
        );
    }

    // ---- Schedule::get_fire_candidate ----------------------------------

    fn cron_schedule(expression: &str) -> Schedule {
        Schedule::Cron {
            expression: expression.to_string(),
            timezone: None,
        }
    }

    #[test]
    fn fire_candidate_cron_returns_most_recent_slot_when_unfired() {
        let now: DateTime<Utc> = "2026-06-03T15:04:05Z".parse().unwrap();
        let slot = cron_schedule("* * * * *")
            .get_fire_candidate(None, now)
            .expect("should be due");
        // Most recent slot ≤ 15:04:05 for "every minute" is 15:04:00.
        assert_eq!(slot.to_rfc3339(), "2026-06-03T15:04:00+00:00");
    }

    #[test]
    fn fire_candidate_cron_returns_now_when_now_is_on_a_slot() {
        let now: DateTime<Utc> = "2026-06-03T15:04:00Z".parse().unwrap();
        let slot = cron_schedule("* * * * *")
            .get_fire_candidate(None, now)
            .expect("should be due");
        assert_eq!(slot, now);
    }

    #[test]
    fn fire_candidate_cron_skips_when_last_fire_is_at_or_after_slot() {
        let now: DateTime<Utc> = "2026-06-03T15:04:05Z".parse().unwrap();
        let last_fire: DateTime<Utc> = "2026-06-03T15:04:00Z".parse().unwrap();
        assert!(
            cron_schedule("* * * * *")
                .get_fire_candidate(Some(last_fire), now)
                .is_none()
        );
    }

    #[test]
    fn fire_candidate_cron_does_not_replay_after_long_downtime() {
        // Last fired 12 minutes before `now`; "every minute" cron.
        // Should fire only the most recent slot, not the missed slots.
        let now: DateTime<Utc> = "2026-06-03T15:12:00Z".parse().unwrap();
        let last_fire: DateTime<Utc> = "2026-06-03T15:00:00Z".parse().unwrap();
        let slot = cron_schedule("* * * * *")
            .get_fire_candidate(Some(last_fire), now)
            .expect("should be due");
        assert_eq!(slot.to_rfc3339(), "2026-06-03T15:12:00+00:00");
    }

    #[test]
    fn fire_candidate_once_returns_at_when_unfired_and_due() {
        let now: DateTime<Utc> = "2026-06-03T15:00:00Z".parse().unwrap();
        let at: DateTime<Utc> = "2026-06-03T14:59:50Z".parse().unwrap();
        let slot = Schedule::Once { at }.get_fire_candidate(None, now).unwrap();
        assert_eq!(slot, at);
    }

    #[test]
    fn fire_candidate_once_skipped_when_already_fired() {
        let now: DateTime<Utc> = "2026-06-03T15:00:00Z".parse().unwrap();
        let at: DateTime<Utc> = "2026-06-03T14:59:50Z".parse().unwrap();
        assert!(
            Schedule::Once { at }
                .get_fire_candidate(Some(at), now)
                .is_none()
        );
    }

    #[test]
    fn fire_candidate_once_skipped_when_in_future() {
        let now: DateTime<Utc> = "2026-06-03T15:00:00Z".parse().unwrap();
        let at: DateTime<Utc> = "2026-06-03T15:00:30Z".parse().unwrap();
        assert!(
            Schedule::Once { at }
                .get_fire_candidate(None, now)
                .is_none()
        );
    }

    #[test]
    fn fire_candidate_cron_returns_none_on_invalid_expression() {
        let now = Utc::now();
        assert!(
            cron_schedule("not a cron")
                .get_fire_candidate(None, now)
                .is_none()
        );
    }

    // ---- Action::run --------------------------------------------------

    mod run_tests {
        use super::*;
        use crate::app::{EventBus, StoreWithEvents};
        use crate::store::{MemoryStore, RelationshipType, Store};
        use hydra_common::ActorId;
        use hydra_common::api::v1::users::Username as ApiUsername;
        use std::sync::Arc;

        fn trigger_actor(trigger_id: TriggerId, creator: &str) -> ActorRef {
            ActorRef::Trigger {
                trigger_id,
                on_behalf_of: Some(ActorId::User(ApiUsername::from(creator))),
            }
        }

        fn wrap(store: Arc<dyn Store>) -> StoreWithEvents {
            StoreWithEvents::new(store, Arc::new(EventBus::new()))
        }

        async fn add_user(store: &dyn Store, username: &str) {
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
            let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
            add_user(inner.as_ref(), "alice").await;
            let store = wrap(inner.clone());
            let trigger_id = TriggerId::new();
            let actor = trigger_actor(trigger_id.clone(), "alice");
            let creator = ApiUsername::from("alice");
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

            let target = run_action(&action, &ctx, &store, &actor, &creator, &trigger_id)
                .await
                .expect("action should succeed");
            let ActionTarget::Issue(issue_id) = target;

            // Issue persisted with rendered fields and the trigger actor.
            let versioned = inner.get_issue(&issue_id, false).await.unwrap();
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
            let edges = inner
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
            let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
            let store = wrap(inner.clone());
            let trigger_id = TriggerId::new();
            let actor = trigger_actor(trigger_id.clone(), "alice");
            let creator = ApiUsername::from("alice");
            let action = create_issue("t", "d", None, None);

            let ctx = RenderContext::new(Utc::now(), Utc::now(), trigger_id.clone());
            let target = run_action(&action, &ctx, &store, &actor, &creator, &trigger_id)
                .await
                .unwrap();
            let ActionTarget::Issue(issue_id) = target;
            let issue = inner.get_issue(&issue_id, false).await.unwrap();
            assert!(issue.item.assignee.is_none());
        }

        #[tokio::test]
        async fn missing_template_variable_returns_render_error() {
            let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
            let store = wrap(inner.clone());
            let trigger_id = TriggerId::new();
            let actor = trigger_actor(trigger_id.clone(), "alice");
            let creator = ApiUsername::from("alice");
            // `{{ bogus }}` is not in `KNOWN_VARIABLES`. The renderer
            // surfaces `RenderError::UnknownVariable`; `Action::run`
            // wraps it in `ActionError::Render`.
            let action = create_issue("hi {{ bogus }}", "d", None, None);

            let ctx = RenderContext::new(Utc::now(), Utc::now(), trigger_id.clone());
            let err = run_action(&action, &ctx, &store, &actor, &creator, &trigger_id)
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
            let listed = inner.list_issues(&Default::default()).await.unwrap();
            assert!(listed.is_empty(), "no issue should be created on failure");
        }

        #[tokio::test]
        async fn unparseable_assignee_returns_parse_error() {
            let inner: Arc<dyn Store> = Arc::new(MemoryStore::new());
            let store = wrap(inner.clone());
            let trigger_id = TriggerId::new();
            let actor = trigger_actor(trigger_id.clone(), "alice");
            let creator = ApiUsername::from("alice");
            // `not-a-principal` is not in the `users/<x>` form.
            let action = create_issue("t", "d", Some("not-a-principal"), None);

            let ctx = RenderContext::new(Utc::now(), Utc::now(), trigger_id.clone());
            let err = run_action(&action, &ctx, &store, &actor, &creator, &trigger_id)
                .await
                .unwrap_err();
            assert!(
                matches!(err, ActionError::ParseAssignee { .. }),
                "got {err:?}"
            );
            let listed = inner.list_issues(&Default::default()).await.unwrap();
            assert!(listed.is_empty());
        }
    }
}
