//! Integration tests for `hydra graph log` (PR 5 pipe-grammar cutover).
//!
//! Exercises the CLI subcommand end-to-end against the harness's in-memory
//! store + ephemeral HTTP server, covering:
//! - the bare-id fast path (single-object log, no `/v1/relations` call)
//! - `'<id> | neighbors'` for per-version `created`/`updated` events
//! - `'<id> | scope'` (and `| scope | kind=patch`) regression against the
//!   pre-cutover `--scope` invocation
//! - **inclusive-by-default** contract: `| children` over a childless issue
//!   still emits the seed's own events
//! - **`exclusive` flag** regression: matches today's `--source` semantics
//! - `--limit` truncation, descending-ts ordering, `--verbosity` projection,
//!   conversation event-stream fold
//! - **flag removal**: `--source`/`--scope`/`--object`/etc. now produce
//!   clap parse errors at exit 2
//! - PM-playbook smoke: `'<id> | scope'` is the canonical replacement for
//!   the old `--scope <id>` opening invocation

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
async fn log_bare_id_emits_single_object_events() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    // Bare-id source: no `/v1/relations` call is needed at all.
    let issue = user.create_issue("bare-id-log").await?;
    user.update_issue_status(&issue, IssueStatus::InProgress)
        .await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "log",
            issue.as_ref(),
            "--since",
            "-1h",
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    // Should include both the `created` event and the status-update.
    let matched = records_for_id(&records, issue.as_ref());
    assert_eq!(
        matched.len(),
        2,
        "expected 2 events for the lone issue, got {records:?}",
    );
    assert!(
        matched.iter().any(|r| r["event"] == "created"),
        "expected created event in {matched:?}",
    );
    Ok(())
}

#[tokio::test]
async fn log_neighbors_emits_per_version_events_for_issue_in_window() -> Result<()> {
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
            &format!("{} | neighbors", parent.as_ref()),
            "--since",
            "-1h",
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
            &format!("{} | neighbors", parent.as_ref()),
            "--since",
            "-1h",
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
            &format!("{} | neighbors", parent.as_ref()),
            "--since",
            "-1h",
            "--limit",
            "2",
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    assert_eq!(records.len(), 2, "expected --limit 2: {records:?}");
    Ok(())
}

#[tokio::test]
async fn log_scope_form_matches_today_scope_invocation() -> Result<()> {
    // Regression: `'<id> | scope'` is the DSL replacement for today's
    // `--scope <id>` selection.
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("log-scope-parent").await?;
    let child = user.create_child_issue(&parent, "log-scope-child").await?;
    user.update_issue_status(&child, IssueStatus::InProgress)
        .await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "log",
            &format!("{} | scope", parent.as_ref()),
            "--since",
            "-1h",
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    // Both parent and child versions appear because `scope` is inherently
    // inclusive and fans out via child-of.
    assert!(
        !records_for_id(&records, parent.as_ref()).is_empty(),
        "parent missing in scope log: {records:?}",
    );
    assert!(
        !records_for_id(&records, child.as_ref()).is_empty(),
        "child missing in scope log: {records:?}",
    );
    Ok(())
}

#[tokio::test]
async fn log_scope_kind_patch_with_limit() -> Result<()> {
    // `'<id> | scope | kind=patch' --since X --limit 10` — bounded scope
    // patch events.
    use hydra_common::RepoName;
    use std::str::FromStr;
    let harness = harness::TestHarness::builder()
        .with_repo("acme/log-scope-kind")
        .build()
        .await?;
    let user = harness.default_user();
    let client = harness.client()?;
    let repo = RepoName::from_str("acme/log-scope-kind")?;

    let parent = user.create_issue("log-kind-parent").await?;
    let child = user.create_child_issue(&parent, "log-kind-child").await?;
    let patch = user.create_patch("log-kind-p", "x", &repo).await?;
    client
        .create_relation(&CreateRelationRequest {
            source_id: child.clone().into(),
            target_id: patch.clone().into(),
            rel_type: "has-patch".to_string(),
        })
        .await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "log",
            &format!("{} | scope | kind=patch", parent.as_ref()),
            "--since",
            "-1h",
            "--limit",
            "10",
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    assert!(!records.is_empty(), "expected events");
    for record in &records {
        assert_eq!(
            record["kind"].as_str(),
            Some("patch"),
            "kind=patch filter not applied: {record}",
        );
    }
    Ok(())
}

#[tokio::test]
async fn log_inclusive_default_children_includes_seed_with_no_children() -> Result<()> {
    // **Inclusive-by-default** contract: `'<id> | children'` over an issue
    // with no children still emits events for the seed itself.
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let lonely = user.create_issue("log-lonely").await?;
    // No children; no other relations.

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "log",
            &format!("{} | children", lonely.as_ref()),
            "--since",
            "-1h",
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    let matched = records_for_id(&records, lonely.as_ref());
    assert!(
        matched.iter().any(|r| r["event"] == "created"),
        "inclusive-default should keep the seed: {records:?}",
    );
    Ok(())
}

#[tokio::test]
async fn log_exclusive_children_regression_matches_old_source_form() -> Result<()> {
    // `'<id> | children exclusive'` matches today's `--source <id>`: the
    // resolver issues `source_ids=<id>` and drops the seed from the
    // resulting vertex set. For a child-of edge (source=child, target=parent),
    // `'<child> | children exclusive'` walks one hop along the outgoing edge
    // to the parent, then drops the seed.
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("log-excl-parent").await?;
    let child = user.create_child_issue(&parent, "log-excl-child").await?;
    user.update_issue_status(&child, IssueStatus::InProgress)
        .await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "log",
            &format!("{} | children exclusive", child.as_ref()),
            "--since",
            "-1h",
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    // Only the parent should appear; the seed (child) is excluded.
    assert!(
        !records_for_id(&records, parent.as_ref()).is_empty(),
        "expected parent events; got: {records:?}",
    );
    assert!(
        records_for_id(&records, child.as_ref()).is_empty(),
        "exclusive should drop the seed (child); got: {records:?}",
    );
    Ok(())
}

#[tokio::test]
async fn log_conversation_uses_event_fold_via_neighbors_kind_filter() -> Result<()> {
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
            &format!("{} | neighbors | kind=conversation", parent.as_ref()),
            "--since",
            "-1h",
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    // The conversation events log carries only lifecycle events
    // post-Phase-E step 18 (chat content moved to `SessionEvent`), so this
    // scenario produces a single `Closed` event plus the initial `created`
    // version.
    assert!(
        !records.is_empty(),
        "expected fold to produce >=1 event: {records:?}",
    );
    for record in &records {
        assert_eq!(record["kind"].as_str(), Some("conversation"));
        assert_eq!(record["id"].as_str(), Some(conv.conversation_id.as_ref()));
    }
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

    // Anchor the target issue in the relation graph so neighbors of parent
    // resolves to a non-empty node set.
    let parent = user.create_issue("log-v3-parent").await?;
    let issue = user.create_child_issue(&parent, "log-v3-target").await?;
    let existing = client.get_issue(&issue, false).await?;
    let mut updated = existing.issue.clone();
    updated.description = "new description".to_string();
    client
        .update_issue(&issue, &UpsertIssueRequest::new(updated.into(), None))
        .await?;

    let output_l1 = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "log",
            &format!("{} | neighbors", parent.as_ref()),
            "--since",
            "-1h",
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
            &format!("{} | neighbors", parent.as_ref()),
            "--since",
            "-1h",
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
            &format!("{} | neighbors", parent.as_ref()),
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
            &format!("{} | neighbors", parent.as_ref()),
            "--since",
            "2026-05-15T13:00:00Z",
            "--until",
            "2026-05-15T12:00:00Z",
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
            &format!("{} | neighbors", parent.as_ref()),
            "--since",
            "-1h",
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
async fn log_parse_error_exits_code_two() -> Result<()> {
    // Parse failure: `kids` is not a real stage name; the parser surfaces
    // a Levenshtein hint and we exit 2.
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let output = user
        .cli_expect_failure(&["graph", "log", "i-abcdef | kids", "--since", "-1h"])
        .await?;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stderr.contains("unknown stage 'kids'"),
        "expected parser error, got: {}",
        output.stderr,
    );
    assert!(
        output.stderr.contains("children"),
        "expected 'did you mean children?' hint, got: {}",
        output.stderr,
    );
    Ok(())
}

#[tokio::test]
async fn log_old_source_flag_is_rejected_by_clap() -> Result<()> {
    // The `--source` flag (and every other selection flag) was deleted by
    // PR 5. clap rejects it as an unrecognized arg at exit 2.
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();
    let parent = user.create_issue("log-flag-removal").await?;

    let output = user
        .cli_expect_failure(&[
            "graph",
            "log",
            "--source",
            parent.as_ref(),
            "--since",
            "-1h",
        ])
        .await?;
    assert_eq!(output.status.code(), Some(2));
    let stderr_lower = output.stderr.to_lowercase();
    assert!(
        stderr_lower.contains("unexpected argument")
            || stderr_lower.contains("unrecognized argument"),
        "expected clap unknown-arg error, got: {}",
        output.stderr,
    );
    Ok(())
}

#[tokio::test]
async fn log_old_scope_flag_is_rejected_by_clap() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();
    let parent = user.create_issue("log-scope-flag-removal").await?;

    let output = user
        .cli_expect_failure(&["graph", "log", "--scope", parent.as_ref(), "--since", "-1h"])
        .await?;
    assert_eq!(output.status.code(), Some(2));
    let stderr_lower = output.stderr.to_lowercase();
    assert!(
        stderr_lower.contains("unexpected argument")
            || stderr_lower.contains("unrecognized argument"),
        "expected clap unknown-arg error for --scope, got: {}",
        output.stderr,
    );
    Ok(())
}

#[tokio::test]
async fn log_pm_playbook_scope_form_smoke() -> Result<()> {
    // The PM playbook's standard opening invocation maps from
    //   hydra graph log --scope $HYDRA_ISSUE_ID --since -7d --verbosity 2
    // to
    //   hydra graph log '<issue> | scope' --since -7d --verbosity 2
    // This is the exact form the PM agent will use post-cutover.
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let issue = user.create_issue("pm-playbook-smoke").await?;
    let _child = user.create_child_issue(&issue, "pm-playbook-child").await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "log",
            &format!("{} | scope", issue.as_ref()),
            "--since",
            "-7d",
            "--verbosity",
            "2",
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    assert!(
        !records_for_id(&records, issue.as_ref()).is_empty(),
        "PM playbook scope form should surface the seed issue's events: {records:?}",
    );
    Ok(())
}
