use crate::{
    client::HydraClientInterface,
    command::{
        output::{
            render as render_output, CommandContext, DeletedTriggerOutcome, ResolvedOutputFormat,
            TriggerRecords, TriggerTestRecords, TriggerUpsertOutcome,
        },
        utils::resolve_username,
    },
    output_writer::write_stdout,
};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use clap::Subcommand;
use hydra_common::{
    triggers::{
        render as render_template, Action, CreateIssueAction, RenderContext, Schedule,
        SearchTriggersQuery, TriggerVersionRecord, UpsertTriggerRequest, UpsertTriggerResponse,
    },
    users::Username,
    TriggerId,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Subcommand)]
pub enum TriggerCommands {
    /// Create a new trigger from a YAML spec.
    Create {
        /// Path to the YAML file describing the trigger.
        #[arg(long, value_name = "FILE")]
        file: String,
    },
    /// Fetch the latest version of a trigger.
    Get {
        /// Trigger ID to fetch.
        #[arg(value_name = "TRIGGER_ID")]
        id: TriggerId,

        /// Include the trigger even if it has been soft-deleted.
        #[arg(long = "include-deleted")]
        include_deleted: bool,
    },
    /// List triggers.
    List {
        /// Include soft-deleted triggers in the listing.
        #[arg(long = "include-deleted")]
        include_deleted: bool,
    },
    /// Replace a trigger's config from a YAML spec.
    Update {
        /// Trigger ID to update.
        #[arg(value_name = "TRIGGER_ID")]
        id: TriggerId,

        /// Path to the YAML file describing the new trigger config.
        #[arg(long, value_name = "FILE")]
        file: String,
    },
    /// Soft-delete a trigger.
    Delete {
        /// Trigger ID to delete.
        #[arg(value_name = "TRIGGER_ID")]
        id: TriggerId,
    },
    /// Render a trigger's action payloads locally without writing to the
    /// server. Useful for previewing how templates expand at a given time.
    Test {
        /// Path to the YAML file describing the trigger.
        #[arg(long, value_name = "FILE")]
        file: String,

        /// RFC3339 timestamp to pass to the template renderer as
        /// `now`/`scheduled_at`.
        #[arg(long, value_name = "TIMESTAMP")]
        at: DateTime<Utc>,
    },
}

pub async fn run(
    client: &dyn HydraClientInterface,
    command: TriggerCommands,
    context: &CommandContext,
) -> Result<()> {
    match command {
        TriggerCommands::Create { file } => {
            let spec = read_spec(&file)?;
            let creator = resolve_username(client).await?;
            let request = spec.into_request(creator);
            let response = client
                .create_trigger(&request)
                .await
                .context("failed to create trigger")?;
            write_upsert_response(context.output_format, &response)
        }
        TriggerCommands::Get {
            id,
            include_deleted,
        } => {
            let record = client
                .get_trigger(&id, include_deleted)
                .await
                .with_context(|| format!("failed to fetch trigger '{id}'"))?;
            write_trigger_records(context.output_format, &[record])
        }
        TriggerCommands::List { include_deleted } => {
            let query = SearchTriggersQuery {
                include_deleted: include_deleted.then_some(true),
            };
            let response = client
                .list_triggers(&query)
                .await
                .context("failed to list triggers")?;
            write_trigger_records(context.output_format, &response.triggers)
        }
        TriggerCommands::Update { id, file } => {
            let spec = read_spec(&file)?;
            // Carry the trigger's creator forward — `creator` is owned by
            // the trigger and not editable from the YAML spec.
            let current = client
                .get_trigger(&id, false)
                .await
                .with_context(|| format!("failed to fetch trigger '{id}' before update"))?;
            let request = spec.into_request(current.trigger.creator);
            let response = client
                .update_trigger(&id, &request)
                .await
                .with_context(|| format!("failed to update trigger '{id}'"))?;
            write_upsert_response(context.output_format, &response)
        }
        TriggerCommands::Delete { id } => {
            let deleted = client
                .delete_trigger(&id)
                .await
                .with_context(|| format!("failed to delete trigger '{id}'"))?;
            let mut buffer = Vec::new();
            render_output(
                DeletedTriggerOutcome(&deleted.trigger_id),
                context.output_format,
                &mut buffer,
            )?;
            write_stdout(&buffer)?;
            Ok(())
        }
        TriggerCommands::Test { file, at } => {
            let spec = read_spec(&file)?;
            run_test(spec, at, context.output_format)
        }
    }
}

/// On-the-wire YAML spec for `hydra triggers create|update|test`. The
/// `creator` field on the wire-format `Trigger` is set server-side from
/// auth, so it is intentionally absent here — the CLI injects it from
/// `whoami` (or from the current trigger row, in the `update` case).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct TriggerSpec {
    #[serde(default = "default_enabled")]
    enabled: bool,
    schedule: Schedule,
    #[serde(default)]
    actions: Vec<Action>,
}

fn default_enabled() -> bool {
    true
}

impl TriggerSpec {
    fn into_request(self, creator: Username) -> UpsertTriggerRequest {
        UpsertTriggerRequest::new(self.enabled, self.schedule, self.actions, creator)
    }
}

fn read_spec(path: &str) -> Result<TriggerSpec> {
    let yaml = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read trigger spec file '{path}'"))?;
    parse_spec(&yaml).with_context(|| format!("failed to parse trigger spec YAML in '{path}'"))
}

/// Parse the YAML spec via a JSON intermediate so externally-tagged enum
/// variants (`Cron: { ... }`, `Once: { at: ... }`, `CreateIssue: { ... }`)
/// can be written as map keys — `serde_yaml_ng`'s direct path requires the
/// less ergonomic YAML tag form (`!Cron`).
fn parse_spec(yaml: &str) -> Result<TriggerSpec> {
    let yaml_value: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(yaml).context("YAML is not well-formed")?;
    let json_value: serde_json::Value =
        serde_json::to_value(yaml_value).context("YAML could not be normalised to JSON")?;
    let spec: TriggerSpec = serde_json::from_value(json_value)
        .context("trigger spec does not match the expected shape")?;
    Ok(spec)
}

fn write_trigger_records(
    format: ResolvedOutputFormat,
    records: &[TriggerVersionRecord],
) -> Result<()> {
    let mut buffer = Vec::new();
    render_output(TriggerRecords(records), format, &mut buffer)?;
    write_stdout(&buffer)?;
    Ok(())
}

fn write_upsert_response(
    format: ResolvedOutputFormat,
    response: &UpsertTriggerResponse,
) -> Result<()> {
    let mut buffer = Vec::new();
    render_output(TriggerUpsertOutcome(response), format, &mut buffer)?;
    write_stdout(&buffer)?;
    Ok(())
}

/// Run the local renderer over each action in `spec` against a synthetic
/// `RenderContext` built from `at`. No server interaction, no auth. The
/// placeholder trigger id `t-aaaaaa` matches the design's example.
fn run_test(spec: TriggerSpec, at: DateTime<Utc>, format: ResolvedOutputFormat) -> Result<()> {
    let placeholder_id = TriggerId::from_str("t-aaaaaa")
        .map_err(|err| anyhow!("internal: placeholder trigger id rejected: {err}"))?;
    let ctx = RenderContext::new(at, at, placeholder_id);

    let rendered: Vec<RenderedAction> = spec
        .actions
        .iter()
        .enumerate()
        .map(|(idx, action)| render_action(idx, action, &ctx))
        .collect::<Result<Vec<_>>>()?;

    let mut buffer = Vec::new();
    render_output(TriggerTestRecords(&rendered), format, &mut buffer)?;
    write_stdout(&buffer)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RenderedAction {
    CreateIssue(RenderedCreateIssue),
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RenderedCreateIssue {
    pub(crate) action_index: usize,
    pub(crate) issue_type: String,
    pub(crate) title: String,
    pub(crate) description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) assignee: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) repo_name: Option<String>,
}

fn render_action(idx: usize, action: &Action, ctx: &RenderContext) -> Result<RenderedAction> {
    match action {
        Action::CreateIssue(create) => {
            let CreateIssueAction {
                issue_type,
                title,
                description,
                assignee,
                status,
                session_settings,
                ..
            } = create;
            let title =
                render_template(title, ctx).map_err(|err| anyhow!("action {idx}: title: {err}"))?;
            let description = render_template(description, ctx)
                .map_err(|err| anyhow!("action {idx}: description: {err}"))?;
            let assignee = match assignee {
                Some(value) => Some(
                    render_template(value, ctx)
                        .map_err(|err| anyhow!("action {idx}: assignee: {err}"))?,
                ),
                None => None,
            };
            Ok(RenderedAction::CreateIssue(RenderedCreateIssue {
                action_index: idx,
                issue_type: issue_type.to_string(),
                title,
                description,
                assignee,
                status: status.as_ref().map(ToString::to_string),
                repo_name: session_settings.repo_name.as_ref().map(ToString::to_string),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::HydraClient;
    use crate::test_utils::ids::trigger_id;
    use httpmock::prelude::*;
    use hydra_common::actor_ref::ActorRef;
    use hydra_common::api::v1::issues::SessionSettings;
    use hydra_common::issues::{IssueStatus, IssueType};
    use hydra_common::triggers::{Schedule, ScheduleFiring, Trigger, TriggerVersionRecord};
    use reqwest::Client as HttpClient;
    use std::str::FromStr;

    const TEST_HYDRA_TOKEN: &str = "test-hydra-token";

    fn hydra_client(server: &MockServer) -> HydraClient {
        HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())
            .unwrap()
    }

    fn sample_trigger() -> Trigger {
        let yaml = sample_yaml();
        let spec: TriggerSpec = parse_spec(yaml).expect("sample yaml must parse");
        Trigger::new(
            spec.enabled,
            spec.schedule,
            spec.actions,
            Username::from("alice"),
            None,
            false,
        )
    }

    fn sample_yaml() -> &'static str {
        r#"
enabled: true
schedule:
  Cron:
    expression: "0 9 * * MON"
    timezone: "America/Los_Angeles"
actions:
  - CreateIssue:
      type: task
      title: "Weekly triage — {{ now.date }}"
      description: "Created by trigger {{ trigger.id }} at {{ now.iso }}"
      assignee: "users/alice"
      status: open
      session_settings:
        repo_name: "dourolabs/hydra"
"#
    }

    fn once_yaml() -> &'static str {
        r#"
schedule:
  Once:
    at: "2026-06-10T09:00:00Z"
actions:
  - CreateIssue:
      type: bug
      title: "one-shot"
      description: "fired"
"#
    }

    #[test]
    fn parse_cron_spec_round_trip() {
        let spec: TriggerSpec = parse_spec(sample_yaml()).expect("parse");
        assert!(spec.enabled);
        match &spec.schedule {
            Schedule::Cron {
                expression,
                timezone,
            } => {
                assert_eq!(expression, "0 9 * * MON");
                assert_eq!(timezone.as_deref(), Some("America/Los_Angeles"));
            }
            other => panic!("expected Cron, got {other:?}"),
        }
        assert_eq!(spec.actions.len(), 1);
        let Action::CreateIssue(action) = &spec.actions[0];
        assert_eq!(action.issue_type, IssueType::Task);
        assert_eq!(action.title, "Weekly triage — {{ now.date }}");
        assert_eq!(action.assignee.as_deref(), Some("users/alice"));
        assert_eq!(action.status, Some(IssueStatus::Open));
        assert_eq!(
            action
                .session_settings
                .repo_name
                .as_ref()
                .map(|r| r.to_string()),
            Some("dourolabs/hydra".to_string()),
        );

        // The wire request the CLI sends to the server preserves all of
        // the spec fields and tacks on the resolved creator.
        let request = spec.into_request(Username::from("alice"));
        assert!(request.enabled);
        assert_eq!(request.creator.as_ref(), "alice");
        assert_eq!(request.actions.len(), 1);
    }

    #[test]
    fn parse_once_spec_round_trip() {
        let spec: TriggerSpec = parse_spec(once_yaml()).expect("parse");
        assert!(spec.enabled, "enabled defaults to true when omitted");
        match &spec.schedule {
            Schedule::Once { at } => {
                let expected: DateTime<Utc> = "2026-06-10T09:00:00Z".parse().unwrap();
                assert_eq!(*at, expected);
            }
            other => panic!("expected Once, got {other:?}"),
        }
    }

    #[test]
    fn parse_rejects_missing_schedule() {
        let yaml = "actions: []\n";
        let err = parse_spec(yaml).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schedule"), "got: {msg}");
    }

    #[test]
    fn test_command_renders_templated_fields() {
        let spec: TriggerSpec = parse_spec(sample_yaml()).expect("parse");
        let at: DateTime<Utc> = "2026-06-10T09:00:00Z".parse().unwrap();
        let placeholder = TriggerId::from_str("t-aaaaaa").unwrap();
        let ctx = RenderContext::new(at, at, placeholder.clone());

        let rendered = spec
            .actions
            .iter()
            .enumerate()
            .map(|(idx, action)| render_action(idx, action, &ctx))
            .collect::<Result<Vec<_>>>()
            .expect("render");
        assert_eq!(rendered.len(), 1);
        let RenderedAction::CreateIssue(action) = &rendered[0];
        assert_eq!(action.title, "Weekly triage — 2026-06-10");
        assert_eq!(
            action.description,
            format!("Created by trigger {placeholder} at 2026-06-10T09:00:00+00:00")
        );
        assert_eq!(action.assignee.as_deref(), Some("users/alice"));
        assert_eq!(action.status.as_deref(), Some("open"));
        assert_eq!(action.repo_name.as_deref(), Some("dourolabs/hydra"));
    }

    #[test]
    fn test_command_surfaces_render_errors() {
        let yaml = r#"
schedule:
  Once:
    at: "2026-06-10T09:00:00Z"
actions:
  - CreateIssue:
      type: task
      title: "hi {{ bogus }}"
      description: "d"
"#;
        let spec: TriggerSpec = parse_spec(yaml).expect("parse");
        let at: DateTime<Utc> = "2026-06-10T09:00:00Z".parse().unwrap();
        let placeholder = TriggerId::from_str("t-aaaaaa").unwrap();
        let ctx = RenderContext::new(at, at, placeholder);
        let err = render_action(0, &spec.actions[0], &ctx).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("action 0"), "got {msg}");
        assert!(msg.contains("title"), "got {msg}");
        assert!(msg.contains("bogus"), "got {msg}");
    }

    #[test]
    fn next_fire_after_for_cron() {
        let schedule = Schedule::Cron {
            expression: "0 9 * * MON".to_string(),
            timezone: None,
        };
        let now: DateTime<Utc> = "2026-06-08T08:59:00Z".parse().unwrap(); // a Monday
        let next = schedule.next_fire_after(now).expect("next slot");
        assert_eq!(next.to_rfc3339(), "2026-06-08T09:00:00+00:00");
    }

    #[test]
    fn next_fire_after_for_once_in_future() {
        let at: DateTime<Utc> = "2026-06-10T09:00:00Z".parse().unwrap();
        let schedule = Schedule::Once { at };
        let now: DateTime<Utc> = "2026-06-09T00:00:00Z".parse().unwrap();
        assert_eq!(schedule.next_fire_after(now), Some(at));
    }

    #[test]
    fn next_fire_after_for_once_in_past_is_none() {
        let at: DateTime<Utc> = "2026-06-10T09:00:00Z".parse().unwrap();
        let schedule = Schedule::Once { at };
        let now: DateTime<Utc> = "2026-06-10T09:00:01Z".parse().unwrap();
        assert!(schedule.next_fire_after(now).is_none());
    }

    #[test]
    fn create_request_serialises_as_externally_tagged() {
        // The wire format the server expects: `{"Cron": {...}}` for
        // `schedule`, `{"CreateIssue": {...}}` for each action, and
        // `"type": "task"` (the rename on `CreateIssueAction.issue_type`).
        let spec: TriggerSpec = parse_spec(sample_yaml()).expect("parse");
        let request = spec.into_request(Username::from("alice"));
        let json = serde_json::to_value(&request).unwrap();
        assert!(json.get("enabled").unwrap().as_bool().unwrap());
        assert_eq!(json["creator"].as_str(), Some("alice"));
        let schedule = &json["schedule"]["Cron"];
        assert_eq!(schedule["expression"].as_str(), Some("0 9 * * MON"));
        assert_eq!(schedule["timezone"].as_str(), Some("America/Los_Angeles"),);
        let action = &json["actions"][0]["CreateIssue"];
        assert_eq!(action["type"].as_str(), Some("task"));
        assert!(action["title"].as_str().unwrap().contains("Weekly triage"));
    }

    #[tokio::test]
    async fn create_get_list_delete_round_trip_against_mock_server() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let tid = trigger_id("t-mockmock");
        let trigger = sample_trigger();

        let create_response = hydra_common::triggers::UpsertTriggerResponse::new(tid.clone(), 1);
        let create_mock = server.mock(|when, then| {
            when.method(POST).path("/v1/triggers");
            then.status(200).json_body_obj(&create_response);
        });
        let request = UpsertTriggerRequest::new(
            trigger.enabled,
            trigger.schedule.clone(),
            trigger.actions.clone(),
            trigger.creator.clone(),
        );
        let resp = client.create_trigger(&request).await.expect("create");
        create_mock.assert();
        assert_eq!(resp.trigger_id, tid);
        assert_eq!(resp.version, 1);

        let record = TriggerVersionRecord::new(
            tid.clone(),
            1,
            chrono::Utc::now(),
            trigger.clone(),
            None::<ActorRef>,
            chrono::Utc::now(),
        );
        let get_mock = server.mock(|when, then| {
            when.method(GET).path(format!("/v1/triggers/{tid}"));
            then.status(200).json_body_obj(&record);
        });
        let fetched = client.get_trigger(&tid, false).await.expect("get");
        get_mock.assert();
        assert_eq!(fetched.trigger_id, tid);

        let list_response = hydra_common::triggers::ListTriggersResponse::new(vec![record.clone()]);
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/triggers");
            then.status(200).json_body_obj(&list_response);
        });
        let listed = client
            .list_triggers(&SearchTriggersQuery::default())
            .await
            .expect("list");
        list_mock.assert();
        assert_eq!(listed.triggers.len(), 1);

        let mut deleted_trigger = trigger;
        deleted_trigger.deleted = true;
        let delete_record = TriggerVersionRecord::new(
            tid.clone(),
            2,
            chrono::Utc::now(),
            deleted_trigger,
            None::<ActorRef>,
            chrono::Utc::now(),
        );
        let delete_mock = server.mock(|when, then| {
            when.method(DELETE).path(format!("/v1/triggers/{tid}"));
            then.status(200).json_body_obj(&delete_record);
        });
        let deleted = client.delete_trigger(&tid).await.expect("delete");
        delete_mock.assert();
        assert!(deleted.trigger.deleted);
    }

    #[tokio::test]
    async fn list_triggers_passes_include_deleted_query() {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let list_response = hydra_common::triggers::ListTriggersResponse::new(Vec::new());
        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/triggers")
                .query_param("include_deleted", "true");
            then.status(200).json_body_obj(&list_response);
        });
        let query = SearchTriggersQuery {
            include_deleted: Some(true),
        };
        client.list_triggers(&query).await.expect("list");
        mock.assert();
    }

    #[test]
    fn upsert_outcome_pretty_matches_legacy_line() {
        let tid = trigger_id("t-mockmock");
        let response = UpsertTriggerResponse::new(tid.clone(), 7);
        let mut buffer = Vec::new();
        render_output(
            TriggerUpsertOutcome(&response),
            ResolvedOutputFormat::Pretty,
            &mut buffer,
        )
        .expect("pretty render");
        let text = String::from_utf8(buffer).expect("utf8");
        assert_eq!(text, format!("Trigger {tid} (version 7)\n"));
    }

    #[test]
    fn upsert_outcome_jsonl_is_single_json_object() {
        let tid = trigger_id("t-mockmock");
        let response = UpsertTriggerResponse::new(tid.clone(), 7);
        let mut buffer = Vec::new();
        render_output(
            TriggerUpsertOutcome(&response),
            ResolvedOutputFormat::Jsonl,
            &mut buffer,
        )
        .expect("jsonl render");
        let text = String::from_utf8(buffer).expect("utf8");
        assert!(text.ends_with('\n'), "jsonl row missing trailing newline");
        let value: serde_json::Value =
            serde_json::from_str(text.trim_end()).expect("jsonl row must parse");
        assert_eq!(value["trigger_id"].as_str(), Some(tid.to_string().as_str()));
        assert_eq!(value["version"].as_u64(), Some(7));
    }

    #[test]
    fn test_records_pretty_empty_shows_placeholder() {
        let rendered: Vec<RenderedAction> = Vec::new();
        let mut buffer = Vec::new();
        render_output(
            TriggerTestRecords(&rendered),
            ResolvedOutputFormat::Pretty,
            &mut buffer,
        )
        .expect("pretty render");
        let text = String::from_utf8(buffer).expect("utf8");
        assert_eq!(text, "(trigger has no actions)\n");
    }

    #[test]
    fn test_records_pretty_renders_all_fields() {
        let spec: TriggerSpec = parse_spec(sample_yaml()).expect("parse");
        let at: DateTime<Utc> = "2026-06-10T09:00:00Z".parse().unwrap();
        let placeholder = TriggerId::from_str("t-aaaaaa").unwrap();
        let ctx = RenderContext::new(at, at, placeholder);
        let rendered = spec
            .actions
            .iter()
            .enumerate()
            .map(|(idx, action)| render_action(idx, action, &ctx))
            .collect::<Result<Vec<_>>>()
            .expect("render");

        let mut buffer = Vec::new();
        render_output(
            TriggerTestRecords(&rendered),
            ResolvedOutputFormat::Pretty,
            &mut buffer,
        )
        .expect("pretty render");
        let text = String::from_utf8(buffer).expect("utf8");
        assert!(text.contains("action 0 create_issue (task)"), "got: {text}");
        assert!(
            text.contains("  title: Weekly triage — 2026-06-10"),
            "got: {text}"
        );
        assert!(text.contains("  description:"), "got: {text}");
        assert!(text.contains("  assignee: users/alice"), "got: {text}");
        assert!(text.contains("  status: open"), "got: {text}");
        assert!(text.contains("  repo: dourolabs/hydra"), "got: {text}");
    }

    #[test]
    fn test_records_pretty_empty_description_shown_as_dash() {
        let yaml = r#"
schedule:
  Once:
    at: "2026-06-10T09:00:00Z"
actions:
  - CreateIssue:
      type: task
      title: "t"
      description: ""
"#;
        let spec: TriggerSpec = parse_spec(yaml).expect("parse");
        let at: DateTime<Utc> = "2026-06-10T09:00:00Z".parse().unwrap();
        let placeholder = TriggerId::from_str("t-aaaaaa").unwrap();
        let ctx = RenderContext::new(at, at, placeholder);
        let rendered = spec
            .actions
            .iter()
            .enumerate()
            .map(|(idx, action)| render_action(idx, action, &ctx))
            .collect::<Result<Vec<_>>>()
            .expect("render");

        let mut buffer = Vec::new();
        render_output(
            TriggerTestRecords(&rendered),
            ResolvedOutputFormat::Pretty,
            &mut buffer,
        )
        .expect("pretty render");
        let text = String::from_utf8(buffer).expect("utf8");
        assert!(text.contains("  description: -"), "got: {text}");
    }

    #[test]
    fn test_records_jsonl_one_object_per_action() {
        let spec: TriggerSpec = parse_spec(sample_yaml()).expect("parse");
        let at: DateTime<Utc> = "2026-06-10T09:00:00Z".parse().unwrap();
        let placeholder = TriggerId::from_str("t-aaaaaa").unwrap();
        let ctx = RenderContext::new(at, at, placeholder);
        let rendered = spec
            .actions
            .iter()
            .enumerate()
            .map(|(idx, action)| render_action(idx, action, &ctx))
            .collect::<Result<Vec<_>>>()
            .expect("render");

        let mut buffer = Vec::new();
        render_output(
            TriggerTestRecords(&rendered),
            ResolvedOutputFormat::Jsonl,
            &mut buffer,
        )
        .expect("jsonl render");
        let text = String::from_utf8(buffer).expect("utf8");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 1);
        let value: serde_json::Value = serde_json::from_str(lines[0]).expect("parse");
        assert_eq!(value["kind"].as_str(), Some("create_issue"));
        assert_eq!(value["issue_type"].as_str(), Some("task"));
        assert_eq!(value["action_index"].as_u64(), Some(0));
    }

    #[test]
    fn test_records_jsonl_empty_input_writes_nothing() {
        let rendered: Vec<RenderedAction> = Vec::new();
        let mut buffer = Vec::new();
        render_output(
            TriggerTestRecords(&rendered),
            ResolvedOutputFormat::Jsonl,
            &mut buffer,
        )
        .expect("jsonl render");
        assert!(buffer.is_empty());
    }

    #[test]
    fn unused_settings_field_keeps_default_session_settings() {
        let yaml = r#"
schedule:
  Once:
    at: "2026-06-10T09:00:00Z"
actions:
  - CreateIssue:
      type: task
      title: "t"
      description: "d"
"#;
        let spec: TriggerSpec = parse_spec(yaml).expect("parse");
        let Action::CreateIssue(action) = &spec.actions[0];
        assert!(SessionSettings::is_default(&action.session_settings));
    }
}
