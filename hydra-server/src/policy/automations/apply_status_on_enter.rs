//! `apply_status_on_enter` — runs an issue's resolved
//! [`StatusDefinition::on_enter`] rule whenever its status changes.
//!
//! The same-issue review hand-off is a direct consequence of this
//! automation: when SWE moves an issue to `in-review`, the resolved
//! status's `on_enter` reassigns to the reviewer agent and attaches the
//! review form; the existing assignee-driven spawn dispatcher picks the
//! reviewer up automatically.
//!
//! Idempotent — re-entering the same status with the resulting state
//! already in place is a no-op (no `upsert_issue`, no version bump).

use async_trait::async_trait;

use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::actors::ActorRef;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};
use hydra_common::api::v1::form::Form;
use hydra_common::api::v1::projects::StatusOnEnter;

const AUTOMATION_NAME: &str = "apply_status_on_enter";

pub struct ApplyStatusOnEnterAutomation;

impl ApplyStatusOnEnterAutomation {
    pub fn new(_params: Option<&serde_yaml_ng::Value>) -> Result<Self, String> {
        Ok(Self)
    }
}

#[async_trait]
impl Automation for ApplyStatusOnEnterAutomation {
    fn name(&self) -> &str {
        AUTOMATION_NAME
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![EventType::IssueCreated, EventType::IssueUpdated],
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        // Skip events this automation triggered to avoid infinite loops.
        if let ActorRef::Automation {
            automation_name, ..
        } = ctx.actor()
        {
            if automation_name == AUTOMATION_NAME {
                return Ok(());
            }
        }

        let (issue_id, payload) = match ctx.event {
            ServerEvent::IssueCreated {
                issue_id, payload, ..
            }
            | ServerEvent::IssueUpdated {
                issue_id, payload, ..
            } => (issue_id, payload),
            _ => return Ok(()),
        };

        let MutationPayload::Issue { old, new, .. } = payload.as_ref() else {
            return Ok(());
        };

        // On update, only fire when the status key actually changed; on
        // create, the issue is entering its initial status for the first
        // time, so always evaluate `on_enter`.
        if let Some(old) = old.as_ref()
            && old.status == new.status
        {
            return Ok(());
        }

        let resolved = match ctx.app_state.resolve_status(new).await {
            Ok(def) => def,
            Err(err) => {
                tracing::warn!(
                    automation = AUTOMATION_NAME,
                    issue_id = %issue_id,
                    status = %new.status,
                    error = %err,
                    "apply_status_on_enter: failed to resolve new status; skipping"
                );
                return Ok(());
            }
        };
        let Some(on_enter) = resolved.on_enter.clone() else {
            return Ok(());
        };

        let StatusOnEnter {
            assign_to,
            attach_form,
            ..
        } = on_enter;

        // Resolve the target form (if any) before mutating the issue.
        //
        // Three failure modes are handled distinctly so a misconfigured
        // fixture doesn't silently drop the `assign_to` half of `on_enter`
        // (see [[i-kjkgdkyu]]):
        //
        // * `NotFound`  — operator misconfiguration. Log a warning and
        //   continue so the assignee is still rewritten; the form simply
        //   stays unattached until the doc is uploaded.
        // * `Transient` — store/IO error. Abort so the automation can be
        //   retried; partial application would leave the assignee changed
        //   but the form perpetually missing.
        // * `Malformed` — the doc exists but is unparseable. Abort and
        //   surface — the config is broken and needs a human.
        let target_form = match attach_form.as_ref() {
            Some(path) => match load_form_from_document(ctx, path.as_str()).await {
                Ok(form) => Some(form),
                Err(FormLoadError::NotFound(msg)) => {
                    tracing::warn!(
                        automation = AUTOMATION_NAME,
                        issue_id = %issue_id,
                        project_id = ?new.project_id,
                        status = %new.status,
                        form_path = %path,
                        reason = %msg,
                        "apply_status_on_enter: form document missing; applying assign_to only"
                    );
                    None
                }
                Err(FormLoadError::Transient(err)) => {
                    return Err(AutomationError::Other(anyhow::anyhow!(
                        "failed to load form '{path}' for issue {issue_id}: {err}"
                    )));
                }
                Err(FormLoadError::Malformed(err)) => {
                    return Err(AutomationError::Other(anyhow::anyhow!(
                        "form '{path}' for issue {issue_id} is malformed: {err}"
                    )));
                }
            },
            None => None,
        };

        // Idempotency: only write when at least one field would actually
        // change. Comparing the *resulting* state to the *current* state
        // makes a no-op the natural outcome of re-entering the same key
        // with the same assignee/form already in place.
        let assignee_changes = assign_to
            .as_ref()
            .is_some_and(|p| Some(p) != new.assignee.as_ref());
        let form_changes = target_form
            .as_ref()
            .is_some_and(|f| Some(f) != new.form.as_ref());
        if !assignee_changes && !form_changes {
            return Ok(());
        }

        let mut updated = new.clone();
        if let Some(principal) = assign_to {
            updated.assignee = Some(principal);
        }
        if let Some(form) = target_form {
            updated.form = Some(form);
        }

        let actor = ActorRef::Automation {
            automation_name: AUTOMATION_NAME.into(),
            triggered_by: Some(Box::new(ctx.actor().clone())),
        };

        if let Err(err) = ctx
            .app_state
            .upsert_issue(
                Some(issue_id.clone()),
                hydra_common::api::v1::issues::UpsertIssueRequest::new(updated.into(), None),
                actor,
            )
            .await
        {
            return Err(AutomationError::Other(anyhow::anyhow!(
                "failed to apply on_enter for issue {issue_id}: {err}"
            )));
        }

        tracing::info!(
            automation = AUTOMATION_NAME,
            issue_id = %issue_id,
            new_status = %new.status,
            "applied on_enter automation"
        );

        Ok(())
    }
}

/// Categorises why `load_form_from_document` failed, so the caller can
/// choose between "log and continue", "abort and retry", and "abort and
/// surface" without sniffing error strings.
#[derive(Debug)]
enum FormLoadError {
    /// The document does not exist (or has been deleted) at the given path.
    /// Treated as operator misconfiguration: the automation continues with
    /// the remaining on_enter actions.
    NotFound(String),
    /// An underlying store/IO error occurred. The automation aborts so the
    /// framework can retry the event.
    Transient(anyhow::Error),
    /// The document exists but its body is not a valid form. Aborts and
    /// surfaces so an operator can fix the doc.
    Malformed(anyhow::Error),
}

/// Look up a document by path and parse its body as a YAML [`Form`].
/// This is the same wire format the CLI accepts via `--form`.
async fn load_form_from_document(
    ctx: &AutomationContext<'_>,
    path: &str,
) -> Result<Form, FormLoadError> {
    use crate::store::StoreError;

    let store = ctx.app_state.store();
    let doc_id = store
        .find_non_deleted_document_by_exact_path(path)
        .await
        .map_err(|e| {
            FormLoadError::Transient(anyhow::anyhow!("store error reading '{path}': {e}"))
        })?
        .ok_or_else(|| FormLoadError::NotFound(format!("no document at path '{path}'")))?;
    let doc = match store.get_document(&doc_id, false).await {
        Ok(versioned) => versioned.item,
        Err(StoreError::DocumentNotFound(_)) => {
            return Err(FormLoadError::NotFound(format!(
                "document '{path}' was deleted"
            )));
        }
        Err(err) => {
            return Err(FormLoadError::Transient(anyhow::anyhow!(
                "store error reading '{path}': {err}"
            )));
        }
    };

    let form: Form = serde_yaml_ng::from_str(&doc.body_markdown)
        .map_err(|e| FormLoadError::Malformed(anyhow::anyhow!("not valid YAML: {e}")))?;
    form.validate_field_keys()
        .map_err(|e| FormLoadError::Malformed(anyhow::anyhow!("invalid fields: {e}")))?;
    Ok(form)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::event_bus::MutationPayload;
    use crate::domain::actors::ActorRef;
    use crate::domain::documents::Document;
    use crate::domain::issues::{Issue, IssueStatus, IssueType};
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use crate::test_utils;
    use chrono::Utc;
    use hydra_common::api::v1::projects::{
        IconKey, Project, ProjectKey, StatusDefinition, StatusKey, StatusOnEnter,
    };
    use hydra_common::principal::Principal;
    use std::sync::Arc;

    fn make_issue(status: IssueStatus) -> Issue {
        Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "desc".to_string(),
            Username::from("worker"),
            String::new(),
            status.into(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        )
    }

    fn status_def_with_on_enter(
        key: &str,
        unblocks_parents: bool,
        on_enter: Option<StatusOnEnter>,
    ) -> StatusDefinition {
        StatusDefinition::new(
            StatusKey::try_new(key).unwrap(),
            key.to_string(),
            IconKey::try_new("circle").unwrap(),
            "#abcdef".parse().unwrap(),
            unblocks_parents,
            unblocks_parents,
            false,
            on_enter,
        )
    }

    async fn build_engineering_project(
        handles: &test_utils::TestStateHandles,
        on_enter_for_in_review: StatusOnEnter,
    ) -> hydra_common::ProjectId {
        let statuses = vec![
            status_def_with_on_enter("open", false, None),
            status_def_with_on_enter("in-review", false, Some(on_enter_for_in_review)),
            status_def_with_on_enter("closed", true, None),
        ];
        let project = Project::new(
            ProjectKey::try_new("engineering").unwrap(),
            "Engineering".to_string(),
            statuses,
            StatusKey::try_new("open").unwrap(),
            hydra_common::api::v1::users::Username::try_new("worker").unwrap(),
            false,
        );
        let (id, _) = handles
            .store
            .add_project(project, &ActorRef::test())
            .await
            .unwrap();
        id
    }

    fn issue_updated_event(issue_id: hydra_common::IssueId, old: Issue, new: Issue) -> ServerEvent {
        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old),
            new,
            actor: ActorRef::test(),
        });
        ServerEvent::IssueUpdated {
            seq: 1,
            issue_id,
            version: 2,
            timestamp: Utc::now(),
            payload,
        }
    }

    fn issue_created_event(issue_id: hydra_common::IssueId, new: Issue) -> ServerEvent {
        let payload = Arc::new(MutationPayload::Issue {
            old: None,
            new,
            actor: ActorRef::test(),
        });
        ServerEvent::IssueCreated {
            seq: 1,
            issue_id,
            version: 1,
            timestamp: Utc::now(),
            payload,
        }
    }

    #[tokio::test]
    async fn assign_to_reassigns_issue() {
        let handles = test_utils::test_state_handles();
        let agent = hydra_common::api::v1::agents::AgentName::try_new("reviewer").unwrap();
        // Register the reviewer agent so the assignee passes validation.
        test_utils::add_agent_with_name(&handles, "reviewer").await;

        let project_id = build_engineering_project(
            &handles,
            StatusOnEnter::new(
                Some(Principal::Agent {
                    name: agent.clone(),
                }),
                None,
            ),
        )
        .await;

        let mut issue = make_issue(IssueStatus::Open);
        issue.project_id = Some(project_id);
        let (issue_id, _) = handles
            .store
            .add_issue(issue.clone(), &ActorRef::test())
            .await
            .unwrap();

        let mut transitioned = issue.clone();
        transitioned.status = StatusKey::try_new("in-review").unwrap();
        handles
            .store
            .update_issue(&issue_id, transitioned.clone(), &ActorRef::test())
            .await
            .unwrap();

        let event = issue_updated_event(issue_id.clone(), issue, transitioned);
        let automation = ApplyStatusOnEnterAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };
        automation.execute(&ctx).await.unwrap();

        let stored = handles
            .store
            .get_issue(&issue_id, false)
            .await
            .unwrap()
            .item;
        match stored.assignee {
            Some(Principal::Agent { ref name }) => assert_eq!(name.as_str(), "reviewer"),
            other => panic!("expected reviewer assignee; got {other:?}"),
        }
    }

    #[tokio::test]
    async fn attach_form_replaces_form() {
        let handles = test_utils::test_state_handles();
        // Upload the review form doc the on_enter rule references.
        let doc = Document {
            title: "Review Form".to_string(),
            body_markdown: include_str!("../../../../prompts/forms/review_escalation.yaml")
                .to_string(),
            path: Some("/forms/review_escalation.yaml".parse().unwrap()),
            deleted: false,
        };
        handles
            .store
            .add_document(doc, &ActorRef::test())
            .await
            .unwrap();

        let project_id = build_engineering_project(
            &handles,
            StatusOnEnter::new(None, Some("/forms/review_escalation.yaml".parse().unwrap())),
        )
        .await;

        let mut issue = make_issue(IssueStatus::Open);
        issue.project_id = Some(project_id);
        let (issue_id, _) = handles
            .store
            .add_issue(issue.clone(), &ActorRef::test())
            .await
            .unwrap();

        let mut transitioned = issue.clone();
        transitioned.status = StatusKey::try_new("in-review").unwrap();
        handles
            .store
            .update_issue(&issue_id, transitioned.clone(), &ActorRef::test())
            .await
            .unwrap();

        let event = issue_updated_event(issue_id.clone(), issue, transitioned);
        let automation = ApplyStatusOnEnterAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };
        automation.execute(&ctx).await.unwrap();

        let stored = handles
            .store
            .get_issue(&issue_id, false)
            .await
            .unwrap()
            .item;
        let form = stored.form.expect("form should be attached");
        assert!(
            form.actions.iter().any(|a| a.id == "approve"),
            "review form should declare approve action"
        );
    }

    #[tokio::test]
    async fn no_on_enter_is_noop() {
        let handles = test_utils::test_state_handles();
        let project_id = build_engineering_project(&handles, StatusOnEnter::new(None, None)).await;
        // Override: project's `in-review` status has no on_enter at all.
        // Use `closed` instead which has no on_enter.

        let mut issue = make_issue(IssueStatus::Open);
        issue.project_id = Some(project_id);
        let (issue_id, _) = handles
            .store
            .add_issue(issue.clone(), &ActorRef::test())
            .await
            .unwrap();
        let version_before = handles
            .store
            .get_issue(&issue_id, false)
            .await
            .unwrap()
            .version;

        let mut transitioned = issue.clone();
        transitioned.status = StatusKey::try_new("closed").unwrap();
        handles
            .store
            .update_issue(&issue_id, transitioned.clone(), &ActorRef::test())
            .await
            .unwrap();
        let version_after_transition = handles
            .store
            .get_issue(&issue_id, false)
            .await
            .unwrap()
            .version;
        assert!(version_after_transition > version_before);

        let event = issue_updated_event(issue_id.clone(), issue, transitioned);
        let automation = ApplyStatusOnEnterAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };
        automation.execute(&ctx).await.unwrap();

        let version_final = handles
            .store
            .get_issue(&issue_id, false)
            .await
            .unwrap()
            .version;
        assert_eq!(
            version_final, version_after_transition,
            "no on_enter should not bump version"
        );
    }

    #[tokio::test]
    async fn reentry_with_matching_assignee_is_noop() {
        let handles = test_utils::test_state_handles();
        let agent = hydra_common::api::v1::agents::AgentName::try_new("reviewer").unwrap();
        test_utils::add_agent_with_name(&handles, "reviewer").await;

        let project_id = build_engineering_project(
            &handles,
            StatusOnEnter::new(
                Some(Principal::Agent {
                    name: agent.clone(),
                }),
                None,
            ),
        )
        .await;

        let mut issue = make_issue(IssueStatus::Open);
        issue.project_id = Some(project_id);
        // Pre-assign the issue to the reviewer agent so re-entering
        // `in-review` produces no observable state change.
        issue.assignee = Some(Principal::Agent { name: agent });
        let (issue_id, _) = handles
            .store
            .add_issue(issue.clone(), &ActorRef::test())
            .await
            .unwrap();

        let mut transitioned = issue.clone();
        transitioned.status = StatusKey::try_new("in-review").unwrap();
        handles
            .store
            .update_issue(&issue_id, transitioned.clone(), &ActorRef::test())
            .await
            .unwrap();
        let version_before_automation = handles
            .store
            .get_issue(&issue_id, false)
            .await
            .unwrap()
            .version;

        let event = issue_updated_event(issue_id.clone(), issue, transitioned);
        let automation = ApplyStatusOnEnterAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };
        automation.execute(&ctx).await.unwrap();

        let version_after = handles
            .store
            .get_issue(&issue_id, false)
            .await
            .unwrap()
            .version;
        assert_eq!(
            version_after, version_before_automation,
            "re-entry with matching state should be idempotent"
        );
    }

    #[tokio::test]
    async fn registered_in_default_engine_wiring() {
        // Building the policy engine from the production defaults must
        // include this automation among the active ones. Catches the case
        // where the automation is registered in the registry but never
        // listed in `default_policy_config`.
        use crate::app::default_policy_config;
        let engine = crate::app::AppState::build_policy_engine(Some(&default_policy_config()));
        assert!(
            engine.automation_names().contains(&AUTOMATION_NAME),
            "default engine wiring missing {AUTOMATION_NAME}; got {:?}",
            engine.automation_names()
        );
    }

    #[tokio::test]
    async fn precedes_spawn_sessions_in_default_engine_wiring() {
        use crate::app::default_policy_config;
        let engine = crate::app::AppState::build_policy_engine(Some(&default_policy_config()));
        let names = engine.automation_names();
        let on_enter_idx = names
            .iter()
            .position(|n| *n == AUTOMATION_NAME)
            .expect("apply_status_on_enter registered");
        let spawn_idx = names
            .iter()
            .position(|n| *n == "spawn_sessions")
            .expect("spawn_sessions registered");
        assert!(
            on_enter_idx < spawn_idx,
            "apply_status_on_enter must run before spawn_sessions (so newly-created issues are reassigned per on_enter before the assignment-agent fallback)"
        );
    }

    #[tokio::test]
    async fn issue_created_triggers_on_enter() {
        let handles = test_utils::test_state_handles();
        let agent = hydra_common::api::v1::agents::AgentName::try_new("reviewer").unwrap();
        test_utils::add_agent_with_name(&handles, "reviewer").await;

        // Build a project whose *initial* status `open` has an on_enter
        // rule. A fresh issue lands directly in `open`, so creation should
        // fire the automation.
        let statuses = vec![
            status_def_with_on_enter(
                "open",
                false,
                Some(StatusOnEnter::new(
                    Some(Principal::Agent {
                        name: agent.clone(),
                    }),
                    None,
                )),
            ),
            status_def_with_on_enter("closed", true, None),
        ];
        let project = hydra_common::api::v1::projects::Project::new(
            hydra_common::api::v1::projects::ProjectKey::try_new("engineering").unwrap(),
            "Engineering".to_string(),
            statuses,
            StatusKey::try_new("open").unwrap(),
            hydra_common::api::v1::users::Username::try_new("worker").unwrap(),
            false,
        );
        let (project_id, _) = handles
            .store
            .add_project(project, &ActorRef::test())
            .await
            .unwrap();

        let mut issue = make_issue(IssueStatus::Open);
        issue.project_id = Some(project_id);
        let (issue_id, _) = handles
            .store
            .add_issue(issue.clone(), &ActorRef::test())
            .await
            .unwrap();

        let event = issue_created_event(issue_id.clone(), issue);
        let automation = ApplyStatusOnEnterAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };
        automation.execute(&ctx).await.unwrap();

        let stored = handles
            .store
            .get_issue(&issue_id, false)
            .await
            .unwrap()
            .item;
        match stored.assignee {
            Some(Principal::Agent { ref name }) => assert_eq!(name.as_str(), "reviewer"),
            other => panic!("expected reviewer assignee; got {other:?}"),
        }
    }

    /// When `on_enter` has BOTH `assign_to` AND `attach_form`, the
    /// automation must still rewrite the assignee even if the referenced
    /// form document is missing — the alternative (silently dropping both
    /// halves) is what shipped the in-review regression that motivated
    /// this code path.
    #[tokio::test]
    async fn missing_form_still_applies_assign_to() {
        let handles = test_utils::test_state_handles();
        let agent = hydra_common::api::v1::agents::AgentName::try_new("reviewer").unwrap();
        test_utils::add_agent_with_name(&handles, "reviewer").await;

        // Deliberately do NOT add the form document — the on_enter rule
        // references `/forms/review.yaml`, which is absent.
        let project_id = build_engineering_project(
            &handles,
            StatusOnEnter::new(
                Some(Principal::Agent {
                    name: agent.clone(),
                }),
                Some("/forms/review.yaml".parse().unwrap()),
            ),
        )
        .await;

        let mut issue = make_issue(IssueStatus::Open);
        issue.project_id = Some(project_id);
        let (issue_id, _) = handles
            .store
            .add_issue(issue.clone(), &ActorRef::test())
            .await
            .unwrap();

        let mut transitioned = issue.clone();
        transitioned.status = StatusKey::try_new("in-review").unwrap();
        handles
            .store
            .update_issue(&issue_id, transitioned.clone(), &ActorRef::test())
            .await
            .unwrap();

        let event = issue_updated_event(issue_id.clone(), issue, transitioned);
        let automation = ApplyStatusOnEnterAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };
        automation
            .execute(&ctx)
            .await
            .expect("missing form should not abort the automation");

        let stored = handles
            .store
            .get_issue(&issue_id, false)
            .await
            .unwrap()
            .item;
        match stored.assignee {
            Some(Principal::Agent { ref name }) => assert_eq!(name.as_str(), "reviewer"),
            other => panic!("expected reviewer assignee; got {other:?}"),
        }
        assert!(
            stored.form.is_none(),
            "missing form doc should leave issue.form unattached, got {:?}",
            stored.form
        );
    }

    /// A doc that exists but isn't a parseable form should surface as an
    /// automation error so an operator notices.
    #[tokio::test]
    async fn malformed_form_returns_error() {
        let handles = test_utils::test_state_handles();
        test_utils::add_agent_with_name(&handles, "reviewer").await;

        // Upload a doc at the form path whose body is not a valid form
        // (missing `prompt`, missing `actions`).
        let doc = Document {
            title: "Broken form".to_string(),
            body_markdown: "this: is: not: a form".to_string(),
            path: Some("/forms/review.yaml".parse().unwrap()),
            deleted: false,
        };
        handles
            .store
            .add_document(doc, &ActorRef::test())
            .await
            .unwrap();

        let agent = hydra_common::api::v1::agents::AgentName::try_new("reviewer").unwrap();
        let project_id = build_engineering_project(
            &handles,
            StatusOnEnter::new(
                Some(Principal::Agent { name: agent }),
                Some("/forms/review.yaml".parse().unwrap()),
            ),
        )
        .await;

        let mut issue = make_issue(IssueStatus::Open);
        issue.project_id = Some(project_id);
        let (issue_id, _) = handles
            .store
            .add_issue(issue.clone(), &ActorRef::test())
            .await
            .unwrap();

        let mut transitioned = issue.clone();
        transitioned.status = StatusKey::try_new("in-review").unwrap();
        handles
            .store
            .update_issue(&issue_id, transitioned.clone(), &ActorRef::test())
            .await
            .unwrap();

        let event = issue_updated_event(issue_id.clone(), issue, transitioned);
        let automation = ApplyStatusOnEnterAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };
        let err = automation
            .execute(&ctx)
            .await
            .expect_err("malformed form should surface as an automation error");
        let msg = err.to_string();
        assert!(
            msg.contains("malformed")
                || msg.contains("not valid YAML")
                || msg.contains("invalid fields"),
            "expected malformed-form error, got: {msg}"
        );
    }

    /// `/forms/review.yaml` (shipped under `tests/e2e/fixtures/forms/` and
    /// seeded by `tests/e2e/run.sh`) must parse and validate as a `Form`.
    /// This catches a broken fixture before the e2e harness boots.
    #[test]
    fn shipped_review_form_fixture_parses() {
        let body = include_str!("../../../../tests/e2e/fixtures/forms/review.yaml");
        let form: Form = serde_yaml_ng::from_str(body)
            .expect("review.yaml fixture should deserialize as a Form");
        form.validate_field_keys()
            .expect("review.yaml fixture should have unique field keys");
        assert!(
            form.actions.iter().any(|a| a.id == "approve"),
            "review form should declare an approve action"
        );
        assert!(
            form.actions.iter().any(|a| a.id == "request_changes"),
            "review form should declare a request_changes action"
        );
    }
}
