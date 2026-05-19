//! Integration tests for `hydra graph search`.
//!
//! Exercises the CLI subcommand end-to-end against the harness's in-memory
//! store + ephemeral HTTP server, covering:
//! - basic relation-query selection flags
//! - `--scope` fan-out (and its exclusion of `refers-to`)
//! - `--kind` post-filter
//! - `--verbosity` projection
//! - `--max-nodes` cap (exit 2)
//! - mutually-exclusive flag rejection (exit 2)
//! - `hydra relations list` still works unchanged (PR 6 removes it).

mod harness;

use anyhow::Result;
use hydra_common::api::v1::conversations::CreateConversationRequest;
use hydra_common::api::v1::relations::CreateRelationRequest;
use hydra_common::documents::{Document, UpsertDocumentRequest};
use hydra_common::RepoName;
use serde_json::Value;
use std::str::FromStr;

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

/// Collect node ids from a JSONL graph-search result.
fn node_ids(records: &[Value]) -> Vec<String> {
    let mut ids: Vec<String> = records
        .iter()
        .map(|r| r["id"].as_str().expect("id field").to_string())
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn graph_search_by_object_emits_node_records() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("parent issue").await?;
    let _child = user.create_child_issue(&parent, "child issue").await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "search",
            "--object",
            parent.as_ref(),
        ])
        .await?;

    let records = parse_jsonl(&output.stdout);
    // child-of edge connects child→parent: both endpoints appear in the result.
    let ids = node_ids(&records);
    assert!(
        ids.iter().any(|id| id == parent.as_ref()),
        "parent should appear in results: {ids:?}"
    );
    assert_eq!(
        ids.len(),
        2,
        "expected exactly parent + child node, got {ids:?}"
    );
    for record in &records {
        assert_eq!(record["kind"].as_str(), Some("issue"));
        // L1 default: should have title + status keys
        assert!(record.get("title").is_some(), "L1 missing title: {record}");
        assert!(
            record.get("status").is_some(),
            "L1 missing status: {record}"
        );
        // L3-only fields should NOT appear at L1
        assert!(
            record.get("dependencies").is_none(),
            "dependencies should not appear at L1: {record}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn graph_search_source_transitive_descendants() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let grandparent = user.create_issue("grandparent").await?;
    let parent = user.create_child_issue(&grandparent, "parent").await?;
    let _child = user.create_child_issue(&parent, "child").await?;

    // `--target=grandparent --rel-type=child-of --transitive` returns all
    // descendant edges; union of source/target ids = {grandparent, parent, child}.
    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "search",
            "--target",
            grandparent.as_ref(),
            "--rel-type",
            "child-of",
            "--transitive",
        ])
        .await?;

    let records = parse_jsonl(&output.stdout);
    let ids = node_ids(&records);
    assert_eq!(ids.len(), 3, "expected 3 nodes, got {ids:?}");
    assert!(ids.iter().any(|id| id == grandparent.as_ref()));
    assert!(ids.iter().any(|id| id == parent.as_ref()));
    Ok(())
}

#[tokio::test]
async fn graph_search_scope_covers_descendants_patches_documents() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/graph-search")
        .build()
        .await?;
    let user = harness.default_user();
    let client = harness.client()?;
    let repo = RepoName::from_str("acme/graph-search")?;

    let parent = user.create_issue("scope-parent").await?;
    let child1 = user.create_child_issue(&parent, "scope-child-1").await?;
    let child2 = user.create_child_issue(&parent, "scope-child-2").await?;

    let patch_a = user.create_patch("p1", "child1 patch", &repo).await?;
    let patch_b = user.create_patch("p2", "child2 patch", &repo).await?;

    client
        .create_relation(&CreateRelationRequest {
            source_id: child1.clone().into(),
            target_id: patch_a.clone().into(),
            rel_type: "has-patch".to_string(),
        })
        .await?;
    client
        .create_relation(&CreateRelationRequest {
            source_id: child2.clone().into(),
            target_id: patch_b.clone().into(),
            rel_type: "has-patch".to_string(),
        })
        .await?;

    let doc_a = Document::new(
        "doc-a".to_string(),
        "body".to_string(),
        Some("docs/a.md".to_string()),
        None,
        false,
    )
    .unwrap();
    let doc_a_id = client
        .create_document(&UpsertDocumentRequest::new(doc_a))
        .await?
        .document_id;
    client
        .create_relation(&CreateRelationRequest {
            source_id: child1.clone().into(),
            target_id: doc_a_id.clone().into(),
            rel_type: "has-document".to_string(),
        })
        .await?;

    // Conversation linked via refers-to from parent must NOT be auto-included
    // in --scope (per locked design decision).
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

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "search",
            "--scope",
            parent.as_ref(),
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    let ids = node_ids(&records);

    let expected: Vec<String> = {
        let mut v = vec![
            parent.as_ref().to_string(),
            child1.as_ref().to_string(),
            child2.as_ref().to_string(),
            patch_a.as_ref().to_string(),
            patch_b.as_ref().to_string(),
            doc_a_id.as_ref().to_string(),
        ];
        v.sort();
        v
    };
    assert_eq!(ids, expected, "scope node set mismatch");

    // Conversation must NOT appear.
    let conv_id_str = conv.conversation_id.as_ref().to_string();
    assert!(
        !ids.contains(&conv_id_str),
        "refers-to conversation should not appear under --scope: {ids:?}"
    );

    Ok(())
}

#[tokio::test]
async fn graph_search_scope_with_kind_filter_returns_patches_only() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/graph-search-kind")
        .build()
        .await?;
    let user = harness.default_user();
    let client = harness.client()?;
    let repo = RepoName::from_str("acme/graph-search-kind")?;

    let parent = user.create_issue("kf-parent").await?;
    let _child = user.create_child_issue(&parent, "kf-child").await?;
    let patch = user.create_patch("kf-p", "x", &repo).await?;
    client
        .create_relation(&CreateRelationRequest {
            source_id: parent.clone().into(),
            target_id: patch.clone().into(),
            rel_type: "has-patch".to_string(),
        })
        .await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "search",
            "--scope",
            parent.as_ref(),
            "--kind",
            "patch",
        ])
        .await?;

    let records = parse_jsonl(&output.stdout);
    assert_eq!(records.len(), 1, "expected only patch: {records:?}");
    assert_eq!(records[0]["kind"].as_str(), Some("patch"));
    assert_eq!(records[0]["id"].as_str(), Some(patch.as_ref()));
    Ok(())
}

#[tokio::test]
async fn graph_search_verbosity_three_emits_full_struct() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("v3-parent").await?;
    let _child = user.create_child_issue(&parent, "v3-child").await?;

    let output_l3 = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "search",
            "--object",
            parent.as_ref(),
            "--verbosity",
            "3",
        ])
        .await?;
    let records_l3 = parse_jsonl(&output_l3.stdout);
    let parent_record = records_l3
        .iter()
        .find(|r| r["id"].as_str() == Some(parent.as_ref()))
        .expect("parent record present at L3");

    // L3 = full Issue struct (creator, description, etc.) merged into the
    // top-level node record.
    assert!(
        parent_record.get("description").is_some(),
        "L3 should include full Issue.description: {parent_record}"
    );
    assert!(
        parent_record.get("creator").is_some(),
        "L3 should include creator: {parent_record}"
    );
    Ok(())
}

#[tokio::test]
async fn graph_search_max_nodes_one_exits_code_two() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let grandparent = user.create_issue("mn-grandparent").await?;
    let parent = user.create_child_issue(&grandparent, "mn-parent").await?;
    let _child = user.create_child_issue(&parent, "mn-child").await?;

    let output = user
        .cli_expect_failure(&[
            "--output-format",
            "jsonl",
            "graph",
            "search",
            "--target",
            grandparent.as_ref(),
            "--rel-type",
            "child-of",
            "--transitive",
            "--max-nodes",
            "1",
        ])
        .await?;

    assert_eq!(output.status.code(), Some(2), "expected exit 2");
    assert!(
        output.stderr.contains("narrow your selection"),
        "missing helpful message: {}",
        output.stderr
    );
    Ok(())
}

#[tokio::test]
async fn graph_search_scope_with_source_exits_code_two() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("exclusive-parent").await?;
    let child = user.create_child_issue(&parent, "exclusive-child").await?;

    let output = user
        .cli_expect_failure(&[
            "graph",
            "search",
            "--scope",
            parent.as_ref(),
            "--source",
            child.as_ref(),
        ])
        .await?;

    assert_eq!(output.status.code(), Some(2), "expected exit 2");
    assert!(
        output.stderr.contains("mutually exclusive"),
        "missing mutually-exclusive message: {}",
        output.stderr
    );
    Ok(())
}

#[tokio::test]
async fn graph_search_empty_selection_exits_code_two() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let output = user.cli_expect_failure(&["graph", "search"]).await?;

    assert_eq!(output.status.code(), Some(2), "expected exit 2");
    assert!(
        output
            .stderr
            .contains("at least one of --source, --target, --object, or --scope"),
        "missing helpful message: {}",
        output.stderr
    );
    Ok(())
}

#[tokio::test]
async fn relations_list_still_works() -> Result<()> {
    // Regression: PR 3 does NOT remove `hydra relations list`. PR 6 will.
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("rl-parent").await?;
    let _child = user.create_child_issue(&parent, "rl-child").await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "relations",
            "list",
            "--target",
            parent.as_ref(),
            "--rel-type",
            "child-of",
        ])
        .await?;
    let records = parse_jsonl(&output.stdout);
    assert_eq!(records.len(), 1, "expected one child-of edge");
    assert_eq!(
        records[0]["rel_type"].as_str(),
        Some("child-of"),
        "edge should be child-of"
    );
    Ok(())
}
