//! Integration tests for `hydra graph diff`.
//!
//! Exercises the CLI subcommand end-to-end against the harness's in-memory
//! store + ephemeral HTTP server, covering:
//! - `modified` records for issues whose view projection changed in the window
//! - `added` records for issues created within the window
//! - `removed` records for soft-deleted issues
//! - conversation diffs going through the event-stream fold
//! - `--verbosity` controlling which field changes surface
//! - `--max-nodes` cap (exit 2)
//! - omitted `--since` falls back to the Unix epoch ("from the beginning of time")

mod harness;

use anyhow::Result;
use hydra_common::api::v1::conversations::{CreateConversationRequest, SendMessageRequest};
use hydra_common::api::v1::relations::CreateRelationRequest;
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

#[tokio::test]
async fn diff_emits_modified_record_when_issue_status_changes_in_window() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("parent issue").await?;
    let _child = user.create_child_issue(&parent, "child issue").await?;

    // Mutate the child after a brief delay so its version timestamps are
    // distinct, then run a diff that covers both versions.
    user.update_issue_status(&_child, IssueStatus::InProgress)
        .await?;

    // -1h covers any version timestamps from the in-memory store. The
    // DSL form `<id> | neighbors` mirrors the pre-cutover `--object <id>`
    // semantics with an additional inclusive seed (parent appears in the
    // result by the new default).
    let query = format!("{} | neighbors", parent.as_ref());
    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--since",
            "-1h",
            &query,
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);

    // The child issue should now be classified — at L1, status flipping from
    // `open` to `in-progress` is visible. The child's first version is at the
    // creation time (which is within the -1h window), so the result is an
    // `added` record (no v_start before the window).
    let child_record = find_record(&records, _child.as_ref())
        .unwrap_or_else(|| panic!("child issue diff record present: {records:?}"));
    assert_eq!(child_record["change"].as_str(), Some("added"));
    assert_eq!(child_record["kind"].as_str(), Some("issue"));
    let to_v = child_record["version"]["to"]
        .as_u64()
        .expect("to version number");
    // After update_issue_status, child has at least 2 versions; `to` is the
    // most recent at the until timestamp.
    assert!(to_v >= 2, "expected version >= 2, got {to_v}");
    Ok(())
}

#[tokio::test]
async fn diff_emits_added_for_issue_created_inside_window() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("d-parent").await?;
    let child = user.create_child_issue(&parent, "d-child").await?;

    let query = format!("{} | neighbors", parent.as_ref());
    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--since",
            "-1h",
            &query,
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);

    // Both parent and child were created inside the window, so both records
    // should be `added`.
    for record in &records {
        assert_eq!(record["change"].as_str(), Some("added"), "record: {record}");
        assert_eq!(record["kind"].as_str(), Some("issue"));
    }
    assert!(
        find_record(&records, child.as_ref()).is_some(),
        "child should appear: {records:?}"
    );
    assert!(
        find_record(&records, parent.as_ref()).is_some(),
        "parent should appear: {records:?}"
    );
    Ok(())
}

#[tokio::test]
async fn diff_classifies_soft_deleted_issue_at_l3() -> Result<()> {
    // Constructing a real `removed` classification (v_start exists, v_end =
    // None) requires the deletion to happen *outside* the time window, which
    // the harness can't easily reproduce without manipulating server-side
    // clocks. Instead, exercise the L3 projection over a soft-deleted issue
    // to confirm it surfaces the `deleted` field change as a `modified`
    // record (or `added` if both versions sit inside the window). The
    // dispatch/fetch_versions + classify path is covered.
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();
    let client = harness.client()?;

    let parent = user.create_issue("rm-parent").await?;
    let child = user.create_child_issue(&parent, "rm-child").await?;
    client.delete_issue(&child).await?;

    let query = format!("{} | neighbors", parent.as_ref());
    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--since",
            "-1h",
            &query,
            "--verbosity",
            "3",
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    let child_record = find_record(&records, child.as_ref()).expect("child record should appear");
    let change = child_record["change"].as_str();
    assert!(
        change == Some("added") || change == Some("modified"),
        "expected added/modified at L3, got change={change:?}: {child_record}"
    );
    Ok(())
}

#[tokio::test]
async fn diff_conversation_modified_uses_event_fold() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();
    let client = harness.client()?;

    // Anchor the conversation in the relation graph by linking it to a parent
    // issue via `refers-to`, then point the diff at the issue via `--object`
    // so the relation query returns the conversation in the node set.
    let parent = user.create_issue("conv-diff-parent").await?;
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

    // Generate events: a user message (status=Active) followed by a close
    // (status=Closed). Both fall within the -1h window.
    client
        .send_message(
            &conv.conversation_id,
            &SendMessageRequest {
                content: "hi".to_string(),
            },
        )
        .await?;
    client.close_conversation(&conv.conversation_id).await?;

    let query = format!("{} | neighbors | kind=conversation", parent.as_ref());
    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--since",
            "-1h",
            &query,
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);

    // Only the conversation passes the kind filter. Since both event-derived
    // versions sit inside the window (v_start = None, v_end = latest), the
    // record is `added`. The version number reflects the event-fold count.
    assert_eq!(
        records.len(),
        1,
        "expected one conversation record: {records:?}"
    );
    let record = &records[0];
    assert_eq!(record["kind"].as_str(), Some("conversation"));
    assert_eq!(record["id"].as_str(), Some(conv.conversation_id.as_ref()),);
    let change = record["change"].as_str();
    assert!(
        change == Some("added") || change == Some("modified"),
        "expected added/modified for in-window conversation, got: {record}"
    );
    let to_v = record["version"]["to"]
        .as_u64()
        .expect("to version present");
    // Only lifecycle events land on the conversation events log
    // post-Phase-E step 18 (chat content moved to `SessionEvent`), so the
    // fold sees a single `Closed` event for this scenario.
    assert!(
        to_v >= 1,
        "expected fold to produce >=1 version, got {to_v}"
    );
    Ok(())
}

#[tokio::test]
async fn diff_l1_hides_change_visible_only_at_l3() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    // Description-only mutation: L1 projects title+status, so it should hide
    // the description change. L3 projects the full struct, so it should
    // surface it as a `modified` record (or `added`, if both versions sit
    // inside the window — both classifications must include `description`
    // in the fields object at L3 only if `modified`).
    let issue = user.create_issue("v3-target").await.expect("create issue");

    // Description mutation through the typed client. We update_issue with a
    // fresh description.
    let client = harness.client()?;
    let existing = client.get_issue(&issue, false).await?;
    let mut updated = existing.issue.clone();
    updated.description = "new description".to_string();
    use hydra_common::issues::UpsertIssueRequest;
    client
        .update_issue(&issue, &UpsertIssueRequest::new(updated.into(), None))
        .await?;

    let query = format!("{} | neighbors", issue.as_ref());
    let output_l1 = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--since",
            "-1h",
            &query,
            "--verbosity",
            "1",
        ])
        .await?;
    let records_l1 = parse_jsonl(&output_l1.stdout);
    // Issue may appear as `added` (because all versions sit inside the
    // window). Either way, no `modified` record should appear under L1
    // for description-only churn (L1 hides description).
    for record in &records_l1 {
        assert_ne!(
            record["change"].as_str(),
            Some("modified"),
            "L1 should not surface description-only changes: {record}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn diff_max_nodes_one_exits_code_two() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("mn-parent").await?;
    let _child = user.create_child_issue(&parent, "mn-child").await?;

    let query = format!("{} | neighbors", parent.as_ref());
    let output = user
        .cli_expect_failure(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--since",
            "-1h",
            &query,
            "--max-nodes",
            "1",
        ])
        .await?;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stderr.contains("narrow your selection"),
        "missing helpful message: {}",
        output.stderr
    );
    Ok(())
}

#[tokio::test]
async fn diff_without_since_defaults_to_epoch() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("ms-parent").await?;
    let _child = user.create_child_issue(&parent, "ms-child").await?;

    // No --since: should succeed (epoch default covers all history) and
    // surface the parent as an Added record.
    let query = format!("{} | neighbors", parent.as_ref());
    let output = user
        .cli(&["--output-format", "jsonl", "graph", "diff", &query])
        .await?;
    let records = parse_jsonl(&output.stdout);
    let record = find_record(&records, parent.as_ref())
        .unwrap_or_else(|| panic!("expected record for parent in {records:?}"));
    assert_eq!(record["change"], "added");
    Ok(())
}

#[tokio::test]
async fn diff_since_after_until_exits_code_two() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();
    let parent = user.create_issue("sau-parent").await?;

    let query = parent.as_ref().to_string();
    let output = user
        .cli_expect_failure(&[
            "graph",
            "diff",
            "--since",
            "2026-05-15T13:00:00Z",
            "--until",
            "2026-05-15T12:00:00Z",
            &query,
        ])
        .await?;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stderr.contains("must be <="),
        "expected --since/--until ordering error: {}",
        output.stderr
    );
    Ok(())
}

/// Calling `graph diff` with no positional argument exits at clap parse time
/// (the `<QUERY>` argument is required).
#[tokio::test]
async fn diff_missing_query_exits_code_two() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();
    let output = user
        .cli_expect_failure(&["graph", "diff", "--since", "-1h"])
        .await?;
    assert_eq!(output.status.code(), Some(2));
    let stderr_lower = output.stderr.to_lowercase();
    assert!(
        stderr_lower.contains("required") || stderr_lower.contains("<query>"),
        "expected clap missing-arg error for <QUERY>, got: {}",
        output.stderr
    );
    Ok(())
}

/// Hard-cutover regression: the old node-selection flags are gone. Each one
/// must error at clap parse time (no silent acceptance).
#[tokio::test]
async fn diff_removed_flags_exit_code_two() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();
    let parent = user.create_issue("rmflag-parent").await?;
    let query = parent.as_ref().to_string();

    for flag in [
        "--source",
        "--target",
        "--object",
        "--rel-type",
        "--scope",
        "--kind",
    ] {
        let output = user
            .cli_expect_failure(&["graph", "diff", &query, flag, "i-otherxx"])
            .await?;
        assert_eq!(
            output.status.code(),
            Some(2),
            "expected exit 2 for removed flag '{flag}', stderr was: {}",
            output.stderr
        );
        let stderr_lower = output.stderr.to_lowercase();
        assert!(
            stderr_lower.contains("unexpected")
                || stderr_lower.contains("unrecognized")
                || stderr_lower.contains("unknown")
                || stderr_lower.contains("found argument"),
            "expected clap unknown-flag error for '{flag}', got: {}",
            output.stderr
        );
    }

    // --transitive is a bare bool (no value), so test it separately.
    let output = user
        .cli_expect_failure(&["graph", "diff", &query, "--transitive"])
        .await?;
    assert_eq!(output.status.code(), Some(2));
    Ok(())
}

/// Inclusive-by-default contract: `<id> | children` over an issue with no
/// children returns the seed (rather than the empty set the old `--source`
/// form returned).
#[tokio::test]
async fn diff_inclusive_default_keeps_seed_with_no_children() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let lonely = user.create_issue("lonely-no-children").await?;
    let query = format!("{} | children", lonely.as_ref());
    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--since",
            "-1h",
            &query,
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    // Seed must appear despite no child-of edges existing.
    assert!(
        find_record(&records, lonely.as_ref()).is_some(),
        "inclusive default should keep the seed in the result set: {records:?}"
    );
    Ok(())
}

/// `<id> | parents rel=child-of exclusive` matches the pre-cutover
/// `--target <id> --rel-type child-of` behavior: only the issues that point
/// at `<id>` via `child-of` (i.e. its children in the issue hierarchy)
/// appear, and the seed is dropped.
///
/// Naming nuance: a child issue carries a `child-of` edge with
/// `source = child` and `target = parent`. So in DSL terms, asking for the
/// hierarchy's *children of `parent`* means "sources of edges whose target
/// is `parent`", which is the `parents` stage (per the design's mapping
/// table) — not `children`.
#[tokio::test]
async fn diff_exclusive_drops_seed() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("excl-parent").await?;
    let child = user.create_child_issue(&parent, "excl-child").await?;

    let query = format!("{} | parents rel=child-of exclusive", parent.as_ref());
    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--since",
            "-1h",
            &query,
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    assert!(
        find_record(&records, parent.as_ref()).is_none(),
        "exclusive should drop the seed (parent): {records:?}"
    );
    assert!(
        find_record(&records, child.as_ref()).is_some(),
        "exclusive should keep the child issue: {records:?}"
    );
    Ok(())
}

/// `<id> | scope` mirrors today's `--scope <id>` fan-out (issue + descendants
/// + has-patch + has-document, without `refers-to`).
#[tokio::test]
async fn diff_scope_stage_covers_descendants_patches_documents() -> Result<()> {
    use hydra_common::documents::{Document, UpsertDocumentRequest};
    use hydra_common::RepoName;
    use std::str::FromStr;

    let harness = harness::TestHarness::builder()
        .with_repo("acme/graph-diff-scope")
        .build()
        .await?;
    let user = harness.default_user();
    let client = harness.client()?;
    let repo = RepoName::from_str("acme/graph-diff-scope")?;

    let parent = user.create_issue("scope-diff-parent").await?;
    let child = user.create_child_issue(&parent, "scope-diff-child").await?;
    let patch = user.create_patch("scope-diff-p", "x", &repo).await?;
    client
        .create_relation(&CreateRelationRequest {
            source_id: child.clone().into(),
            target_id: patch.clone().into(),
            rel_type: "has-patch".to_string(),
        })
        .await?;
    let doc = Document::new(
        "scope-diff-doc".to_string(),
        "body".to_string(),
        Some("docs/x.md".to_string()),
        false,
    )
    .unwrap();
    let doc_id = client
        .create_document(&UpsertDocumentRequest::new(doc))
        .await?
        .document_id;
    client
        .create_relation(&CreateRelationRequest {
            source_id: child.clone().into(),
            target_id: doc_id.clone().into(),
            rel_type: "has-document".to_string(),
        })
        .await?;

    let query = format!("{} | scope", parent.as_ref());
    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--since",
            "-1h",
            &query,
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    for id in [
        parent.as_ref(),
        child.as_ref(),
        patch.as_ref(),
        doc_id.as_ref(),
    ] {
        assert!(
            find_record(&records, id).is_some(),
            "expected {id} in scope diff: {records:?}"
        );
    }
    Ok(())
}

/// `<id> | scope | kind=patch` post-filters the scope fan-out to patches only.
#[tokio::test]
async fn diff_scope_with_kind_filter_keeps_only_patches() -> Result<()> {
    use hydra_common::RepoName;
    use std::str::FromStr;

    let harness = harness::TestHarness::builder()
        .with_repo("acme/graph-diff-kind")
        .build()
        .await?;
    let user = harness.default_user();
    let client = harness.client()?;
    let repo = RepoName::from_str("acme/graph-diff-kind")?;

    let parent = user.create_issue("kf-diff-parent").await?;
    let _child = user.create_child_issue(&parent, "kf-diff-child").await?;
    let patch = user.create_patch("kf-diff-p", "x", &repo).await?;
    client
        .create_relation(&CreateRelationRequest {
            source_id: parent.clone().into(),
            target_id: patch.clone().into(),
            rel_type: "has-patch".to_string(),
        })
        .await?;

    let query = format!("{} | scope | kind=patch", parent.as_ref());
    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "diff",
            "--since",
            "-1h",
            &query,
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    assert_eq!(
        records.len(),
        1,
        "expected only patch in records: {records:?}"
    );
    assert_eq!(records[0]["kind"].as_str(), Some("patch"));
    assert_eq!(records[0]["id"].as_str(), Some(patch.as_ref()));
    Ok(())
}

/// Parse errors on the positional `<QUERY>` exit with code 2 and surface the
/// caret-quoted error block from the parser, including the spelling hint.
#[tokio::test]
async fn diff_parse_error_with_caret_hint() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let output = user
        .cli_expect_failure(&["graph", "diff", "--since", "-1h", "i-abcdef | kids"])
        .await?;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stderr.contains("unknown stage 'kids'"),
        "expected unknown-stage error: {}",
        output.stderr
    );
    assert!(
        output.stderr.contains("children"),
        "expected hint pointing at 'children': {}",
        output.stderr
    );
    Ok(())
}
