//! Integration tests for `/v1/triggers` and the
//! `ScheduledTriggerWorker`.
//!
//! The worker is exercised by calling `run_iteration` directly so
//! tests do not need to sleep through real 10s ticks.

use crate::{
    background::scheduled_triggers::ScheduledTriggerWorker,
    background::scheduler::{ScheduledWorker, WorkerOutcome},
    test_utils::{spawn_test_server, test_client, test_state_handles},
};
use chrono::Utc;
use hydra_common::{
    api::v1::{
        issues::{IssueType, ListIssuesResponse, SessionSettings},
        triggers::{
            Action, ListTriggersResponse, Schedule, TriggerVersionRecord, UpsertTriggerRequest,
            UpsertTriggerResponse,
        },
        users::Username,
    },
    test_utils::status::status,
    triggers::Trigger,
};
use reqwest::StatusCode;
use serde_json::Value;

fn sample_request(schedule: Schedule) -> UpsertTriggerRequest {
    UpsertTriggerRequest::new(
        true,
        schedule,
        vec![Action::CreateIssue {
            issue_type: IssueType::Task,
            title: "Triage {{ now.date }}".to_string(),
            description: "Trigger {{ trigger.id }}".to_string(),
            assignee: None,
            project_id: crate::domain::projects::default_project_id(),
            status: status("open"),
            session_settings: SessionSettings::default(),
        }],
        Username::from("test-creator"),
    )
}

#[tokio::test]
async fn post_then_get_round_trip() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let created: UpsertTriggerResponse = client
        .post(format!("{}/v1/triggers", server.base_url()))
        .json(&sample_request(Schedule::Cron {
            expression: "0 9 * * MON".to_string(),
            timezone: Some("UTC".to_string()),
        }))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(created.version, 1);

    let fetched: TriggerVersionRecord = client
        .get(format!(
            "{}/v1/triggers/{}",
            server.base_url(),
            created.trigger_id
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(fetched.trigger_id, created.trigger_id);
    assert!(fetched.trigger.enabled);

    let listed: ListTriggersResponse = client
        .get(format!("{}/v1/triggers", server.base_url()))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(listed.triggers.len(), 1);

    Ok(())
}

#[tokio::test]
async fn post_rejects_invalid_cron() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let response = client
        .post(format!("{}/v1/triggers", server.base_url()))
        .json(&sample_request(Schedule::Cron {
            expression: "not a cron".to_string(),
            timezone: None,
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn post_rejects_unknown_template_variable() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let mut request = sample_request(Schedule::Cron {
        expression: "* * * * *".to_string(),
        timezone: None,
    });
    if let Action::CreateIssue { ref mut title, .. } = request.actions[0] {
        *title = "hi {{ bogus }}".to_string();
    } else {
        panic!("expected CreateIssue variant");
    }

    let response = client
        .post(format!("{}/v1/triggers", server.base_url()))
        .json(&request)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn post_rejects_unknown_repo_name() -> anyhow::Result<()> {
    // `session_settings.repo_name` references a repo the store does not
    // know about. `AppState::create_trigger` performs a targeted
    // `Store::get_repository` lookup and rejects the write with 400.
    let server = spawn_test_server().await?;
    let client = test_client();

    let mut request = sample_request(Schedule::Cron {
        expression: "* * * * *".to_string(),
        timezone: None,
    });
    if let Action::CreateIssue {
        ref mut session_settings,
        ..
    } = request.actions[0]
    {
        session_settings.repo_name = Some(std::str::FromStr::from_str("acme/unknown").unwrap());
    } else {
        panic!("expected CreateIssue variant");
    }

    let response = client
        .post(format!("{}/v1/triggers", server.base_url()))
        .json(&request)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = response.json().await?;
    let message = body
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(
        message.contains("unknown_repo") || message.contains("acme/unknown"),
        "body should mention unknown repo: {message}"
    );
    Ok(())
}

#[tokio::test]
async fn post_warns_on_past_once() -> anyhow::Result<()> {
    // Past-Once is a warning, not a rejection.
    let server = spawn_test_server().await?;
    let client = test_client();
    let past: chrono::DateTime<Utc> = "2020-01-01T00:00:00Z".parse().unwrap();
    let response = client
        .post(format!("{}/v1/triggers", server.base_url()))
        .json(&sample_request(Schedule::Once { at: past }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn delete_marks_trigger_deleted_and_filters_from_list() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let created: UpsertTriggerResponse = client
        .post(format!("{}/v1/triggers", server.base_url()))
        .json(&sample_request(Schedule::Cron {
            expression: "* * * * *".to_string(),
            timezone: None,
        }))
        .send()
        .await?
        .json()
        .await?;

    let response = client
        .delete(format!(
            "{}/v1/triggers/{}",
            server.base_url(),
            created.trigger_id
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let listed: ListTriggersResponse = client
        .get(format!("{}/v1/triggers", server.base_url()))
        .send()
        .await?
        .json()
        .await?;
    assert!(
        listed.triggers.is_empty(),
        "archived trigger must be hidden"
    );

    // include_archived=true surfaces the tombstoned row.
    let listed_with: ListTriggersResponse = client
        .get(format!(
            "{}/v1/triggers?include_archived=true",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(listed_with.triggers.len(), 1);
    assert!(listed_with.triggers[0].trigger.archived);
    Ok(())
}

/// End-to-end: POST a `Schedule::Once { at: past }` trigger via HTTP,
/// drive the worker by calling `run_iteration()` once, then assert
/// exactly one rendered issue + one `created` edge.
#[tokio::test]
async fn worker_fires_once_trigger_creates_issue_and_edge() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let store = handles.store.clone();
    let state = handles.state.clone();

    // Build a trigger directly via the store (faster than the HTTP
    // route; the HTTP layer is covered by the routes tests above).
    let trigger = Trigger::new(
        true,
        Schedule::Once {
            at: Utc::now() - chrono::Duration::seconds(1),
        },
        vec![Action::CreateIssue {
            issue_type: IssueType::Task,
            title: "Triage {{ now.date }}".to_string(),
            description: "Trigger {{ trigger.id }}".to_string(),
            assignee: None,
            project_id: crate::domain::projects::default_project_id(),
            status: status("open"),
            session_settings: SessionSettings::default(),
        }],
        Username::from("test-creator"),
        None,
        false,
    );
    let (trigger_id, version) = store
        .add_trigger(trigger, &crate::domain::actors::ActorRef::test())
        .await?;
    assert_eq!(version, 1);

    let worker = ScheduledTriggerWorker::new(state.clone());
    let outcome = worker.run_iteration().await;
    assert!(
        matches!(
            outcome,
            WorkerOutcome::Progress {
                processed: 1,
                failed: 0
            }
        ),
        "got {outcome:?}"
    );

    // Issue created.
    let issues = store.list_issues(&Default::default()).await?;
    assert_eq!(issues.len(), 1, "exactly one issue should be created");
    let (issue_id, issue_v) = issues.into_iter().next().unwrap();
    assert!(issue_v.item.title.starts_with("Triage "));
    assert!(issue_v.item.description.starts_with("Trigger "));

    // `created` edge present.
    let edges = store
        .get_relationships(
            Some(&hydra_common::HydraId::from(trigger_id.clone())),
            Some(&hydra_common::HydraId::from(issue_id)),
            Some(crate::store::RelationshipType::Created),
        )
        .await?;
    assert_eq!(edges.len(), 1);

    // last_fired_at persisted but no new version row.
    let after = store.get_trigger(&trigger_id, false).await?;
    assert_eq!(after.version, 1, "fire must not bump version");
    assert!(after.item.last_fired_at.is_some());

    // A second tick should not refire the Once trigger.
    let outcome2 = worker.run_iteration().await;
    assert!(matches!(outcome2, WorkerOutcome::Idle), "got {outcome2:?}");
    let after_second = store.list_issues(&Default::default()).await?;
    assert_eq!(after_second.len(), 1, "Once must not refire");

    Ok(())
}

#[tokio::test]
async fn worker_idle_with_no_triggers() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let worker = ScheduledTriggerWorker::new(handles.state);
    let outcome = worker.run_iteration().await;
    assert!(matches!(outcome, WorkerOutcome::Idle));
    Ok(())
}

#[tokio::test]
async fn get_trigger_versions_returns_full_history() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let created: UpsertTriggerResponse = client
        .post(format!("{}/v1/triggers", server.base_url()))
        .json(&sample_request(Schedule::Cron {
            expression: "* * * * *".to_string(),
            timezone: None,
        }))
        .send()
        .await?
        .json()
        .await?;

    // Drive an update so the trigger has two versions on file.
    let mut update_body = sample_request(Schedule::Cron {
        expression: "0 9 * * *".to_string(),
        timezone: None,
    });
    update_body.enabled = false;
    let _: UpsertTriggerResponse = client
        .put(format!(
            "{}/v1/triggers/{}",
            server.base_url(),
            created.trigger_id
        ))
        .json(&update_body)
        .send()
        .await?
        .json()
        .await?;

    let versions: Value = client
        .get(format!(
            "{}/v1/triggers/{}/versions",
            server.base_url(),
            created.trigger_id
        ))
        .send()
        .await?
        .json()
        .await?;
    let arr = versions
        .get("versions")
        .and_then(|v| v.as_array())
        .expect("versions array");
    assert_eq!(arr.len(), 2, "expected v1 and v2: {arr:?}");
    let v1 = &arr[0];
    let v2 = &arr[1];
    assert_eq!(v1.get("version").and_then(|v| v.as_i64()), Some(1));
    assert_eq!(v2.get("version").and_then(|v| v.as_i64()), Some(2));
    Ok(())
}

#[tokio::test]
async fn worker_skips_disabled_triggers() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let store = handles.store.clone();
    let trigger = Trigger::new(
        false, // disabled
        Schedule::Once {
            at: Utc::now() - chrono::Duration::seconds(1),
        },
        vec![Action::CreateIssue {
            issue_type: IssueType::Task,
            title: "t".to_string(),
            description: "d".to_string(),
            assignee: None,
            project_id: crate::domain::projects::default_project_id(),
            status: status("open"),
            session_settings: SessionSettings::default(),
        }],
        Username::from("test-creator"),
        None,
        false,
    );
    store
        .add_trigger(trigger, &crate::domain::actors::ActorRef::test())
        .await?;

    let worker = ScheduledTriggerWorker::new(handles.state);
    let outcome = worker.run_iteration().await;
    assert!(matches!(outcome, WorkerOutcome::Idle));
    let issues = store.list_issues(&Default::default()).await?;
    assert!(issues.is_empty());
    Ok(())
}

/// A server restart followed by a worker iteration must not refire a
/// slot whose `last_fired_at >= scheduled_at`.
#[tokio::test]
async fn worker_does_not_refire_after_restart() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let store = handles.store.clone();
    let state = handles.state.clone();

    // Cron fires daily at 09:00 UTC. `last_fired_at = now` ensures the
    // most recent slot has already been recorded — the next slot is
    // tomorrow at 09:00, well in the future.
    let last_fired: chrono::DateTime<Utc> = Utc::now();
    let trigger = Trigger::new(
        true,
        Schedule::Cron {
            expression: "0 9 * * *".to_string(),
            timezone: None,
        },
        vec![Action::CreateIssue {
            issue_type: IssueType::Task,
            title: "t".to_string(),
            description: "d".to_string(),
            assignee: None,
            project_id: crate::domain::projects::default_project_id(),
            status: status("open"),
            session_settings: SessionSettings::default(),
        }],
        Username::from("test-creator"),
        Some(last_fired),
        false,
    );
    let (trigger_id, _) = store
        .add_trigger(trigger, &crate::domain::actors::ActorRef::test())
        .await?;

    let worker = ScheduledTriggerWorker::new(state);
    let _ = worker.run_iteration().await;

    // No issue should be created because the most recent minute slot
    // has not yet elapsed past `last_fired`. (The next slot for "* * *
    // * *" after `last_fired` is the next exact minute boundary.)
    let response: ListIssuesResponse = test_client_state(&store).await?;
    assert!(
        response.issues.is_empty(),
        "must not refire a slot already covered by last_fired_at; got {:?}",
        response.issues
    );

    // Trigger row still version 1.
    let v = store.get_trigger(&trigger_id, false).await?;
    assert_eq!(v.version, 1);
    Ok(())
}

/// Helper: list_issues against the store directly, returning the API
/// response shape so the assertion is more legible.
async fn test_client_state(
    store: &std::sync::Arc<dyn crate::store::Store>,
) -> anyhow::Result<ListIssuesResponse> {
    let issues = store.list_issues(&Default::default()).await?;
    let issues = issues
        .into_iter()
        .map(|(id, versioned)| {
            let input: hydra_common::api::v1::issues::IssueInput = versioned.item.into();
            // The summary assertions only inspect identity-level fields,
            // not the status definition's display props, so use the shared
            // test helper rather than round-tripping through the project store.
            let resolved = hydra_common::test_utils::status::make_status_def(input.status.clone());
            let api_issue = hydra_common::api::v1::issues::Issue::new(
                input.issue_type,
                input.title,
                input.description,
                input.creator,
                resolved,
                input.project_id,
                input.assignee,
                Some(input.session_settings),
                input.dependencies,
                input.patches,
                input.archived,
                input.form,
                input.form_response,
            );
            let summary = hydra_common::api::v1::issues::IssueSummary::from(&api_issue);
            hydra_common::api::v1::issues::IssueSummaryRecord::new(
                id,
                versioned.version,
                versioned.timestamp,
                summary,
                versioned.actor,
                versioned.creation_time,
                vec![],
            )
        })
        .collect();
    Ok(ListIssuesResponse::new(issues))
}
