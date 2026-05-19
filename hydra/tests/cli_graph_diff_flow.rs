//! Integration tests for `hydra graph diff`.
//!
//! Exercises the CLI subcommand end-to-end against the harness's in-memory
//! store + ephemeral HTTP server. Time windows are constructed from real
//! version timestamps so the tests do not depend on `sleep` calibration.

mod harness;

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use hydra_common::api::v1::conversations::CreateConversationRequest;
use hydra_common::issues::IssueStatus;
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

fn find_record<'a>(records: &'a [Value], id: &str) -> Option<&'a Value> {
    records.iter().find(|r| r["id"].as_str() == Some(id))
}

fn rfc3339(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339()
}

#[tokio::test]
async fn diff_modified_issue_emits_modified_record_with_field_diff() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();
    let client = harness.client()?;

    let parent = user.create_issue("diff-parent").await?;
    let _child = user.create_child_issue(&parent, "diff-child").await?;

    // Mutate the parent issue's status; this produces version 2.
    user.update_issue_status(&parent, IssueStatus::InProgress)
        .await?;

    // Use the recorded version timestamps so the test is deterministic.
    let versions = client.list_issue_versions(&parent).await?;
    assert_eq!(versions.versions.len(), 2, "expected v1 and v2");
    let v1 = &versions.versions[0];
    let v2 = &versions.versions[1];
    // Mid-point timestamp picks v1 as v_start and v2 as v_end.
    let since = v1.timestamp + (v2.timestamp - v1.timestamp) / 2;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--object",
            parent.as_ref(),
            "--since",
            &rfc3339(since),
        ])
        .await?;

    let records = parse_jsonl(&output.stdout);
    let parent_record = find_record(&records, parent.as_ref())
        .unwrap_or_else(|| panic!("expected parent diff record, got: {records:?}"));
    assert_eq!(parent_record["change"].as_str(), Some("modified"));
    assert_eq!(parent_record["kind"].as_str(), Some("issue"));
    assert_eq!(parent_record["version"]["from"].as_u64(), Some(1));
    assert_eq!(parent_record["version"]["to"].as_u64(), Some(2));
    let fields = parent_record["fields"]
        .as_object()
        .expect("fields map present");
    let status_change = fields
        .get("status")
        .expect("status field should show in L1 diff");
    assert_eq!(status_change["before"].as_str(), Some("open"));
    assert_eq!(status_change["after"].as_str(), Some("in-progress"));
    Ok(())
}

#[tokio::test]
async fn diff_newly_created_issue_emits_added_record() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let t_before = Utc::now() - Duration::try_minutes(5).unwrap();
    let parent = user.create_issue("added-parent").await?;
    let _child = user.create_child_issue(&parent, "added-child").await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--object",
            parent.as_ref(),
            "--since",
            &rfc3339(t_before),
        ])
        .await?;

    let records = parse_jsonl(&output.stdout);
    let parent_record = find_record(&records, parent.as_ref())
        .unwrap_or_else(|| panic!("expected parent diff record, got: {records:?}"));
    assert_eq!(parent_record["change"].as_str(), Some("added"));
    assert_eq!(parent_record["kind"].as_str(), Some("issue"));
    assert_eq!(parent_record["version"]["to"].as_u64(), Some(1));
    let object = parent_record["object"]
        .as_object()
        .expect("added record carries an L1 object view");
    assert!(
        object.contains_key("title"),
        "object payload should carry the L1 view: {object:?}"
    );
    Ok(())
}

#[tokio::test]
async fn diff_conversation_with_events_emits_modified_via_event_fold() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();
    let client = harness.client()?;

    let t_before = Utc::now() - Duration::try_minutes(5).unwrap();
    let conv = client
        .create_conversation(&CreateConversationRequest {
            message: Some("hi".to_string()),
            agent_name: None,
            session_settings: None,
        })
        .await?;
    // Drive a status transition by closing the conversation. The fold helper
    // turns the close event into a Versioned<Conversation> with
    // `status = Closed`, which the diff should pick up at L1.
    client.close_conversation(&conv.conversation_id).await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--object",
            conv.conversation_id.as_ref(),
            "--since",
            &rfc3339(t_before),
        ])
        .await?;

    // The conversation appears under --object if any refers-to edge exists;
    // without one, the relations query returns no edges. Build a refers-to
    // edge so the selection actually returns the conversation.
    let _ = output;
    let scratch = user.create_issue("conv-anchor").await?;
    client
        .create_relation(&hydra_common::api::v1::relations::CreateRelationRequest {
            source_id: conv.conversation_id.clone().into(),
            target_id: scratch.clone().into(),
            rel_type: "refers-to".to_string(),
        })
        .await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--object",
            scratch.as_ref(),
            "--rel-type",
            "refers-to",
            "--since",
            &rfc3339(t_before),
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    let conv_record = find_record(&records, conv.conversation_id.as_ref())
        .unwrap_or_else(|| panic!("expected conversation diff record, got: {records:?}"));
    // Conversation `created → closed` materialises as either `added`
    // (window starts before creation, no v_start) or `modified` depending on
    // when the first event lands relative to `--since`. Both are valid
    // signals that the event-fold path produced a populated version sequence
    // for the conversation.
    let change = conv_record["change"].as_str().expect("change kind present");
    assert!(
        change == "added" || change == "modified",
        "unexpected change kind for conversation: {conv_record}"
    );
    assert_eq!(conv_record["kind"].as_str(), Some("conversation"));
    Ok(())
}

#[tokio::test]
async fn diff_verbosity_one_hides_field_change_visible_at_verbosity_two() -> Result<()> {
    use hydra_common::issues::UpsertIssueRequest;

    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();
    let client = harness.client()?;

    let parent = user.create_issue("v-parent").await?;
    let _child = user.create_child_issue(&parent, "v-child").await?;

    // Mutate only the `progress` field. This is an L2 field, not an L1 field
    // (L1 is `title` + `status`), so L1 diff should skip the record entirely
    // and L2 diff should classify it as modified.
    let existing = client.get_issue(&parent, false).await?;
    let mut issue = existing.issue;
    issue.progress = "in flight".to_string();
    client
        .update_issue(&parent, &UpsertIssueRequest::new(issue, None))
        .await?;

    let versions = client.list_issue_versions(&parent).await?;
    let v1 = &versions.versions[0];
    let v2 = &versions.versions[1];
    let since = v1.timestamp + (v2.timestamp - v1.timestamp) / 2;

    let output_l1 = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--object",
            parent.as_ref(),
            "--since",
            &rfc3339(since),
            "--verbosity",
            "1",
        ])
        .await?;
    let records_l1 = parse_jsonl(&output_l1.stdout);
    assert!(
        find_record(&records_l1, parent.as_ref()).is_none(),
        "L1 diff should not emit the parent (progress is L2-only): {records_l1:?}"
    );

    let output_l2 = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--object",
            parent.as_ref(),
            "--since",
            &rfc3339(since),
            "--verbosity",
            "2",
        ])
        .await?;
    let records_l2 = parse_jsonl(&output_l2.stdout);
    let parent_record = find_record(&records_l2, parent.as_ref())
        .unwrap_or_else(|| panic!("expected parent in L2 diff, got: {records_l2:?}"));
    assert_eq!(parent_record["change"].as_str(), Some("modified"));
    let fields = parent_record["fields"]
        .as_object()
        .expect("fields object present");
    assert!(
        fields.contains_key("progress"),
        "L2 diff should show progress: {parent_record}"
    );
    Ok(())
}

#[tokio::test]
async fn diff_missing_since_exits_two() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();
    let parent = user.create_issue("missing-since").await?;

    let output = user
        .cli_expect_failure(&[
            "graph",
            "diff",
            "--object",
            parent.as_ref(),
            // intentionally omit --since
        ])
        .await?;
    // clap reports missing required argument with exit code 2.
    assert_eq!(output.status.code(), Some(2), "{}", output.stderr);
    assert!(
        output.stderr.contains("--since"),
        "clap should mention --since: {}",
        output.stderr
    );
    Ok(())
}

#[tokio::test]
async fn diff_since_after_until_exits_two() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();
    let parent = user.create_issue("bad-window").await?;

    let later = Utc::now();
    let earlier = later - Duration::try_hours(1).unwrap();
    let output = user
        .cli_expect_failure(&[
            "graph",
            "diff",
            "--object",
            parent.as_ref(),
            "--since",
            &rfc3339(later),
            "--until",
            &rfc3339(earlier),
        ])
        .await?;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stderr.contains("--since")
            && output.stderr.contains("--until")
            && (output.stderr.contains("at or before") || output.stderr.contains("before")),
        "expected a since-after-until message, got: {}",
        output.stderr
    );
    Ok(())
}

#[tokio::test]
async fn diff_empty_diff_exits_zero_with_no_output() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("empty-diff-parent").await?;
    let _child = user.create_child_issue(&parent, "empty-diff-child").await?;

    // Pick a `--since` that is *after* every existing version, so no node is
    // classified as added/modified/removed.
    let t_future = Utc::now() + Duration::try_hours(1).unwrap();
    let t_far_future = t_future + Duration::try_hours(1).unwrap();
    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--object",
            parent.as_ref(),
            "--since",
            &rfc3339(t_future),
            "--until",
            &rfc3339(t_far_future),
        ])
        .await?;
    assert!(
        output.stdout.trim().is_empty(),
        "expected empty stdout, got: {}",
        output.stdout
    );
    Ok(())
}

#[tokio::test]
async fn diff_max_nodes_one_over_two_node_match_exits_two() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("max-nodes-parent").await?;
    let _child = user.create_child_issue(&parent, "max-nodes-child").await?;

    let t_before = Utc::now() - Duration::try_hours(1).unwrap();
    let output = user
        .cli_expect_failure(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--object",
            parent.as_ref(),
            "--since",
            &rfc3339(t_before),
            "--max-nodes",
            "1",
        ])
        .await?;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stderr.contains("narrow your selection"),
        "expected helpful message, got: {}",
        output.stderr
    );
    Ok(())
}

#[tokio::test]
async fn diff_relative_time_window_form_parses() -> Result<()> {
    // Smoke-test the relative `--since -<N>h` form: it should parse and the
    // command should run to completion (no unwrap panics on the time arg).
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("relative-since").await?;
    let _child = user.create_child_issue(&parent, "relative-child").await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--object",
            parent.as_ref(),
            "--since=-1h",
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    // Parent was just created within the last hour → should appear as added.
    let parent_record = find_record(&records, parent.as_ref())
        .unwrap_or_else(|| panic!("expected parent record, got: {records:?}"));
    assert_eq!(parent_record["change"].as_str(), Some("added"));
    Ok(())
}

#[tokio::test]
async fn diff_bad_time_format_exits_two() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("bad-time").await?;
    let output = user
        .cli_expect_failure(&[
            "graph",
            "diff",
            "--object",
            parent.as_ref(),
            "--since",
            "yesterday",
        ])
        .await?;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stderr.contains("--since"),
        "expected error to call out --since, got: {}",
        output.stderr
    );
    Ok(())
}
