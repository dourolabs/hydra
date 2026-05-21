//! Integration tests for `hydra graph log`.
//!
//! Exercises the CLI subcommand end-to-end against the harness's in-memory
//! store + ephemeral HTTP server, covering:
//! - per-version `created`/`updated` events for an issue with multiple
//!   in-window versions
//! - merging of events across two nodes in descending-ts order
//! - `--limit` truncation
//! - conversation log going through the event-stream fold
//! - `--verbosity` controlling which field changes surface in `changes`
//! - `--since` after `--until` (exit 2)
//! - omitted `--since` falls back to the Unix epoch ("from the beginning of time")
//! - `--max-nodes` cap (exit 2) and empty selection (exit 2)

mod harness;

use anyhow::Result;
use hydra_common::api::v1::conversations::{CreateConversationRequest, SendMessageRequest};
use hydra_common::api::v1::relations::CreateRelationRequest;
use hydra_common::issues::{IssueStatus, UpsertIssueRequest};
use serde_json::Value;

/// Parse stdout as JSONL into a Vec of Value.
fn parse_jsonl(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<Value>(line)
                .unwrap_or_else(|e| panic!("invalid JSON line {line:?}: {e}"))
        })
        .collect()
}

fn records_for_id<'a>(records: &'a [Value], id: &str) -> Vec<&'a Value> {
    records
        .iter()
        .filter(|r| r["id"].as_str() == Some(id))
        .collect()
}

#[tokio::test]
async fn log_emits_per_version_events_for_issue_in_window() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("log-parent").await?;
    let child = user.create_child_issue(&parent, "log-child").await?;
    user.update_issue_status(&child, IssueStatus::InProgress)
        .await?;
    user.update_issue_status(&child, IssueStatus::Closed)
        .await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "log",
            "--since",
            "-1h",
            "--object",
            parent.as_ref(),
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);

    // Each record must have an event kind from {created, updated}.
    for record in &records {
        let event = record["event"].as_str().expect("event field");
        assert!(
            event == "created" || event == "updated",
            "unexpected event kind: {record}",
        );
        assert!(record.get("ts").is_some(), "ts missing: {record}");
        assert!(record.get("version").is_some(), "version missing: {record}");
    }

    let child_events = records_for_id(&records, child.as_ref());
    // Child has at least 3 versions inside the -1h window (create + 2
    // status updates), and the earliest one is the issue's first version, so
    // the events should include exactly one `created`.
    assert!(
        child_events.len() >= 3,
        "expected >= 3 child events, got {child_events:?}",
    );
    let created_count = child_events
        .iter()
        .filter(|e| e["event"].as_str() == Some("created"))
        .count();
    assert_eq!(
        created_count, 1,
        "expected exactly one `created` for child, got events: {child_events:?}",
    );
    Ok(())
}

#[tokio::test]
async fn log_merges_events_across_nodes_in_descending_ts_order() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("log-merge-parent").await?;
    let child_a = user.create_child_issue(&parent, "log-merge-a").await?;
    user.update_issue_status(&child_a, IssueStatus::InProgress)
        .await?;
    let child_b = user.create_child_issue(&parent, "log-merge-b").await?;
    user.update_issue_status(&child_b, IssueStatus::InProgress)
        .await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "log",
            "--since",
            "-1h",
            "--object",
            parent.as_ref(),
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    assert!(!records.is_empty(), "expected events");

    let timestamps: Vec<&str> = records
        .iter()
        .map(|r| r["ts"].as_str().expect("ts field"))
        .collect();
    let mut sorted = timestamps.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(timestamps, sorted, "events not sorted desc-ts: {records:?}");
    // Both children must appear in the merged stream.
    assert!(
        !records_for_id(&records, child_a.as_ref()).is_empty(),
        "child_a missing: {records:?}"
    );
    assert!(
        !records_for_id(&records, child_b.as_ref()).is_empty(),
        "child_b missing: {records:?}"
    );
    Ok(())
}

#[tokio::test]
async fn log_limit_truncates_to_n_records() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("log-limit-parent").await?;
    let child = user.create_child_issue(&parent, "log-limit-child").await?;
    user.update_issue_status(&child, IssueStatus::InProgress)
        .await?;
    user.update_issue_status(&child, IssueStatus::Closed)
        .await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "log",
            "--since",
            "-1h",
            "--object",
            parent.as_ref(),
            "--limit",
            "2",
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    assert_eq!(records.len(), 2, "expected --limit 2: {records:?}");
    Ok(())
}

#[tokio::test]
async fn log_conversation_uses_event_fold() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();
    let client = harness.client()?;

    let parent = user.create_issue("log-conv-parent").await?;
    let conv = client
        .create_conversation(&CreateConversationRequest {
            message: None,
            agent_name: None,
            session_settings: None,
        })
        .await?;
    client
        .create_relation(&CreateRelationRequest {
            source_id: conv.conversation_id.clone().into(),
            target_id: parent.clone().into(),
            rel_type: "refers-to".to_string(),
        })
        .await?;

    client
        .send_message(
            &conv.conversation_id,
            &SendMessageRequest {
                content: "hi".to_string(),
            },
        )
        .await?;
    client.close_conversation(&conv.conversation_id).await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "log",
            "--since",
            "-1h",
            "--object",
            parent.as_ref(),
            "--kind",
            "conversation",
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    // The conversation should produce at least 2 events (the events endpoint
    // yields several events from send_message + close_conversation), all
    // attributed to this conversation id.
    assert!(
        records.len() >= 2,
        "expected fold to produce >=2 events: {records:?}",
    );
    for record in &records {
        assert_eq!(record["kind"].as_str(), Some("conversation"));
        assert_eq!(record["id"].as_str(), Some(conv.conversation_id.as_ref()));
    }
    // Exactly one `created` event (the first version overall is inside the
    // window).
    let created_count = records
        .iter()
        .filter(|r| r["event"].as_str() == Some("created"))
        .count();
    assert_eq!(created_count, 1, "got: {records:?}");
    Ok(())
}

#[tokio::test]
async fn log_l1_hides_description_change_visible_at_l3() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();
    let client = harness.client()?;

    // Anchor the target issue in the relation graph so `--object parent`
    // resolves to a non-empty node set.
    let parent = user.create_issue("log-v3-parent").await?;
    let issue = user.create_child_issue(&parent, "log-v3-target").await?;
    let existing = client.get_issue(&issue, false).await?;
    let mut updated = existing.issue.clone();
    updated.description = "new description".to_string();
    client
        .update_issue(&issue, &UpsertIssueRequest::new(updated, None))
        .await?;

    let output_l1 = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "log",
            "--since",
            "-1h",
            "--object",
            parent.as_ref(),
            "--verbosity",
            "1",
        ])
        .await?;
    let records_l1 = parse_jsonl(&output_l1.stdout);
    assert!(!records_l1.is_empty(), "expected L1 events");
    for record in &records_l1 {
        if record["event"].as_str() == Some("updated") {
            let changes = record["changes"].as_object().expect("changes map");
            assert!(
                !changes.contains_key("description"),
                "L1 should not surface description change: {record}",
            );
        }
    }

    let output_l3 = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "log",
            "--since",
            "-1h",
            "--object",
            parent.as_ref(),
            "--verbosity",
            "3",
        ])
        .await?;
    let records_l3 = parse_jsonl(&output_l3.stdout);
    let surfaced = records_l3.iter().any(|record| {
        record["event"].as_str() == Some("updated")
            && record["id"].as_str() == Some(issue.as_ref())
            && record["changes"]
                .as_object()
                .map(|c| c.contains_key("description"))
                .unwrap_or(false)
    });
    assert!(
        surfaced,
        "L3 should surface description change: {records_l3:?}",
    );
    Ok(())
}

#[tokio::test]
async fn log_without_since_defaults_to_epoch() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("log-ms-parent").await?;
    let _child = user.create_child_issue(&parent, "log-ms-child").await?;

    // No --since: should succeed (epoch default covers all history) and
    // surface the parent's `created` event.
    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "log",
            "--object",
            parent.as_ref(),
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    let matched = records_for_id(&records, parent.as_ref());
    assert!(
        matched.iter().any(|r| r["event"] == "created"),
        "expected created event for parent in {records:?}",
    );
    Ok(())
}

#[tokio::test]
async fn log_since_after_until_exits_code_two() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();
    let parent = user.create_issue("log-sau-parent").await?;

    let output = user
        .cli_expect_failure(&[
            "graph",
            "log",
            "--since",
            "2026-05-15T13:00:00Z",
            "--until",
            "2026-05-15T12:00:00Z",
            "--object",
            parent.as_ref(),
        ])
        .await?;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stderr.contains("must be <="),
        "expected --since/--until ordering error: {}",
        output.stderr,
    );
    Ok(())
}

#[tokio::test]
async fn log_max_nodes_one_exits_code_two() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();
    let parent = user.create_issue("log-mn-parent").await?;
    let _child = user.create_child_issue(&parent, "log-mn-child").await?;

    let output = user
        .cli_expect_failure(&[
            "--output-format",
            "jsonl",
            "graph",
            "log",
            "--since",
            "-1h",
            "--object",
            parent.as_ref(),
            "--max-nodes",
            "1",
        ])
        .await?;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stderr.contains("narrow your selection"),
        "missing helpful message: {}",
        output.stderr,
    );
    Ok(())
}

#[tokio::test]
async fn log_empty_selection_exits_code_two() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();
    let output = user
        .cli_expect_failure(&["graph", "log", "--since", "-1h"])
        .await?;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        output
            .stderr
            .contains("at least one of --source, --target, --object, or --scope"),
        "missing helpful message: {}",
        output.stderr,
    );
    Ok(())
}
