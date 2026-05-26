//! Integration tests for `hydra graph search`.
//!
//! Exercises the CLI subcommand end-to-end against the harness's in-memory
//! store + ephemeral HTTP server. After PR 3 (`hydra graph i-fqdipnqf`), the
//! selection input is the positional pipe-grammar query parsed in
//! `hydra_common::graph::query`. Coverage:
//!
//! - bare-id fast path (no `/v1/relations` call expected).
//! - `| neighbors` over a single seed (today's `--object`).
//! - `| descendants rel=child-of` ≡ `| children rel=child-of transitive`
//!   (today's `--source X --rel-type child-of --transitive`).
//! - `| ancestors rel=child-of` ≡ `| parents rel=child-of transitive`
//!   (today's `--target X --rel-type child-of --transitive`).
//! - `| children` over a leaf (default-inclusive contract: seed is kept
//!   even when the traversal returns zero rows — new behavior).
//! - `| parents rel=child-of exclusive` (today's "just the children" set).
//! - `| scope` (regression against today's `--scope` algorithm).
//! - `| scope | kind=patch` (post-hydration kind filter).
//! - multi-element source `i-x, i-y | scope`.
//! - parse error surfacing with caret block.
//! - the removed `--source` / `--target` / `--object` / `--rel-type` /
//!   `--transitive` / `--scope` / `--kind` flags now error at clap parse time.
//! - `--max-nodes` cap (exit 2).
//! - the legacy `relations` top-level CLI subcommand is gone.

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
async fn graph_search_bare_id_hydrates_seed_only() -> Result<()> {
    // The bare-id fast path: a single id with no stages emits exactly one
    // hydrated record (the seed itself). No `--scope` fan-out, no relations
    // call.
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("bare-id-parent").await?;
    let _child = user.create_child_issue(&parent, "bare-id-child").await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "search",
            parent.as_ref(),
        ])
        .await?;

    let records = parse_jsonl(&output.stdout);
    assert_eq!(
        records.len(),
        1,
        "bare id should hydrate only itself: {records:?}"
    );
    assert_eq!(records[0]["id"].as_str(), Some(parent.as_ref()));
    assert_eq!(records[0]["kind"].as_str(), Some("issue"));
    Ok(())
}

#[tokio::test]
async fn graph_search_neighbors_exclusive_matches_object_semantics() -> Result<()> {
    // Regression against today's `--object i-parent`: returns the
    // child + parent set with the `child-of` edge between them. Today's
    // form excluded the seed only when `--object` had no edges. Under the
    // new DSL, `neighbors exclusive` always drops the seed.
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("parent issue").await?;
    let child = user.create_child_issue(&parent, "child issue").await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "search",
            &format!("{} | neighbors", parent.as_ref()),
        ])
        .await?;

    let records = parse_jsonl(&output.stdout);
    let ids = node_ids(&records);
    assert!(
        ids.iter().any(|id| id == parent.as_ref()),
        "parent should appear in inclusive results: {ids:?}",
    );
    assert!(
        ids.iter().any(|id| id == child.as_ref()),
        "child should appear: {ids:?}",
    );
    assert_eq!(
        ids.len(),
        2,
        "expected exactly parent + child node, got {ids:?}"
    );
    for record in &records {
        assert_eq!(record["kind"].as_str(), Some("issue"));
        assert!(record.get("title").is_some(), "L1 missing title: {record}");
        assert!(
            record.get("status").is_some(),
            "L1 missing status: {record}"
        );
        assert!(
            record.get("dependencies").is_none(),
            "L3-only field should not appear at L1: {record}",
        );
    }
    Ok(())
}

#[tokio::test]
async fn graph_search_descendants_sugar_matches_children_transitive() -> Result<()> {
    // Per the design doc mapping, today's `--source i-x --rel-type child-of
    // --transitive` maps to `i-x | children rel=child-of transitive` or the
    // sugar `i-x | descendants rel=child-of`. For child-of edges (source =
    // child, target = parent), this traversal walks UP the tree from i-x to
    // its ancestors. We use the leaf (`child`) as the seed so the traversal
    // has rows to return.
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let grandparent = user.create_issue("grandparent").await?;
    let parent = user.create_child_issue(&grandparent, "parent").await?;
    let child = user.create_child_issue(&parent, "child").await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "search",
            &format!("{} | descendants rel=child-of", child.as_ref()),
        ])
        .await?;
    let ids = node_ids(&parse_jsonl(&output.stdout));
    assert_eq!(
        ids.len(),
        3,
        "expected child + parent + grandparent, got {ids:?}"
    );
    assert!(ids.iter().any(|id| id == grandparent.as_ref()));
    assert!(ids.iter().any(|id| id == parent.as_ref()));
    assert!(ids.iter().any(|id| id == child.as_ref()));

    // The explicit `children rel=child-of transitive` form must agree with
    // the sugar.
    let output2 = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "search",
            &format!("{} | children rel=child-of transitive", child.as_ref()),
        ])
        .await?;
    let ids2 = node_ids(&parse_jsonl(&output2.stdout));
    assert_eq!(ids, ids2, "descendants sugar should equal explicit form");
    Ok(())
}

#[tokio::test]
async fn graph_search_ancestors_sugar_walks_subtree_below() -> Result<()> {
    // Today's `--target grandparent --rel-type child-of --transitive` maps to
    // `i-grandparent | parents rel=child-of transitive` or sugar
    // `i-grandparent | ancestors rel=child-of`. For child-of edges this
    // walks DOWN the tree from grandparent through every descendant.
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let grandparent = user.create_issue("grandparent").await?;
    let parent = user.create_child_issue(&grandparent, "parent").await?;
    let child = user.create_child_issue(&parent, "child").await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "search",
            &format!("{} | ancestors rel=child-of", grandparent.as_ref()),
        ])
        .await?;
    let ids = node_ids(&parse_jsonl(&output.stdout));
    assert_eq!(
        ids.len(),
        3,
        "expected grandparent + parent + child, got {ids:?}"
    );
    assert!(ids.iter().any(|id| id == grandparent.as_ref()));
    assert!(ids.iter().any(|id| id == parent.as_ref()));
    assert!(ids.iter().any(|id| id == child.as_ref()));

    // Sugar must equal the explicit `parents rel=child-of transitive` form.
    let output2 = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "search",
            &format!("{} | parents rel=child-of transitive", grandparent.as_ref()),
        ])
        .await?;
    let ids2 = node_ids(&parse_jsonl(&output2.stdout));
    assert_eq!(ids, ids2, "ancestors sugar should equal explicit form");
    Ok(())
}

#[tokio::test]
async fn graph_search_children_default_inclusive_preserves_seed_with_no_results() -> Result<()> {
    // Inclusive-by-default contract: when the traversal returns zero rows,
    // the seed is still present in the output. This is **new** behavior —
    // the prior flag surface would have returned the empty set.
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let lonely = user.create_issue("lonely issue").await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "search",
            &format!("{} | children", lonely.as_ref()),
        ])
        .await?;
    let ids = node_ids(&parse_jsonl(&output.stdout));
    assert_eq!(ids, vec![lonely.as_ref().to_string()], "inclusive seed");
    Ok(())
}

#[tokio::test]
async fn graph_search_parents_exclusive_returns_children_in_tree() -> Result<()> {
    // child-of edges have source=child, target=parent. To collect the
    // children of a parent issue in the tree (today's `--target i-parent
    // --rel-type child-of`), the DSL form is `parents rel=child-of`:
    // direction = Target, so we query `target_ids=parent`, get sources back,
    // and the sources are the children. `exclusive` drops the parent seed
    // and gives us the exact "just the children" set.
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("ex-parent").await?;
    let child1 = user.create_child_issue(&parent, "ex-child-1").await?;
    let child2 = user.create_child_issue(&parent, "ex-child-2").await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "search",
            &format!("{} | parents rel=child-of exclusive", parent.as_ref()),
        ])
        .await?;

    let ids = node_ids(&parse_jsonl(&output.stdout));
    assert!(
        !ids.iter().any(|id| id == parent.as_ref()),
        "exclusive drops seed: {ids:?}"
    );
    assert!(ids.iter().any(|id| id == child1.as_ref()));
    assert!(ids.iter().any(|id| id == child2.as_ref()));
    assert_eq!(ids.len(), 2, "expected only the two children: {ids:?}");
    Ok(())
}

#[tokio::test]
async fn graph_search_scope_covers_descendants_patches_documents() -> Result<()> {
    // Regression against today's `--scope <id>` fixture. The conversation
    // linked via refers-to must NOT auto-appear.
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
            &format!("{} | scope", parent.as_ref()),
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

    let conv_id_str = conv.conversation_id.as_ref().to_string();
    assert!(
        !ids.contains(&conv_id_str),
        "refers-to conversation should not appear under scope: {ids:?}",
    );

    Ok(())
}

#[tokio::test]
async fn graph_search_csv_source_scope_unions_two_scopes() -> Result<()> {
    // `i-x, i-y | scope` distributes the 3-call scope expansion over both
    // seeds and emits the union.
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent_a = user.create_issue("csv-parent-a").await?;
    let child_a = user.create_child_issue(&parent_a, "csv-child-a").await?;
    let parent_b = user.create_issue("csv-parent-b").await?;
    let child_b = user.create_child_issue(&parent_b, "csv-child-b").await?;

    let output = user
        .cli(&[
            "--output-format",
            "jsonl",
            "graph",
            "search",
            &format!("{}, {} | scope", parent_a.as_ref(), parent_b.as_ref()),
        ])
        .await?;
    let ids = node_ids(&parse_jsonl(&output.stdout));

    let mut expected = vec![
        parent_a.as_ref().to_string(),
        child_a.as_ref().to_string(),
        parent_b.as_ref().to_string(),
        child_b.as_ref().to_string(),
    ];
    expected.sort();
    assert_eq!(ids, expected, "csv-source scope union mismatch");
    Ok(())
}

#[tokio::test]
async fn graph_search_scope_with_kind_filter_returns_patches_only() -> Result<()> {
    // `kind=` runs after hydration — i.e. as a post-filter on the resolved
    // set, not as a relation-stage filter on the edge query.
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
            &format!("{} | scope | kind=patch", parent.as_ref()),
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
            &format!("{} | neighbors", parent.as_ref()),
            "--verbosity",
            "3",
        ])
        .await?;
    let records_l3 = parse_jsonl(&output_l3.stdout);
    let parent_record = records_l3
        .iter()
        .find(|r| r["id"].as_str() == Some(parent.as_ref()))
        .expect("parent record present at L3");

    assert!(
        parent_record.get("description").is_some(),
        "L3 should include full Issue.description: {parent_record}",
    );
    assert!(
        parent_record.get("creator").is_some(),
        "L3 should include creator: {parent_record}",
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

    // `ancestors rel=child-of` walks DOWN the tree from grandparent through
    // every descendant via child-of (3 nodes); --max-nodes 1 must reject.
    let output = user
        .cli_expect_failure(&[
            "--output-format",
            "jsonl",
            "graph",
            "search",
            &format!("{} | ancestors rel=child-of", grandparent.as_ref()),
            "--max-nodes",
            "1",
        ])
        .await?;

    assert_eq!(output.status.code(), Some(2), "expected exit 2");
    assert!(
        output.stderr.contains("narrow your selection"),
        "missing helpful message: {}",
        output.stderr,
    );
    Ok(())
}

#[tokio::test]
async fn graph_search_parse_error_renders_caret_hint() -> Result<()> {
    // `kids` is a Levenshtein-≤2 typo for `children`; the parser's caret
    // block + hint must surface to stderr.
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent = user.create_issue("parse-err-parent").await?;

    let output = user
        .cli_expect_failure(&["graph", "search", &format!("{} | kids", parent.as_ref())])
        .await?;

    assert_ne!(output.status.code(), Some(0), "expected non-zero exit");
    assert!(
        output.stderr.contains("unknown stage 'kids'"),
        "missing parse-error header: {}",
        output.stderr,
    );
    assert!(
        output.stderr.contains("did you mean 'children'?"),
        "missing levenshtein hint: {}",
        output.stderr,
    );
    assert!(
        output.stderr.contains("^"),
        "missing caret block: {}",
        output.stderr,
    );
    Ok(())
}

/// Each removed flag must now error at clap parse time. We assert a
/// non-zero exit and that clap mentions the unexpected argument.
async fn assert_flag_removed(flag: &str, value: Option<&str>) -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();
    let parent = user.create_issue("flag-removed").await?;

    let mut args = vec![
        "graph".to_string(),
        "search".to_string(),
        parent.as_ref().to_string(),
        flag.to_string(),
    ];
    if let Some(v) = value {
        args.push(v.to_string());
    }
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    let output = user.cli_expect_failure(&arg_refs).await?;
    assert_eq!(output.status.code(), Some(2), "expected exit 2 for {flag}");
    let stderr_lower = output.stderr.to_lowercase();
    assert!(
        stderr_lower.contains("unexpected argument")
            || stderr_lower.contains("unrecognized argument")
            || stderr_lower.contains("found argument"),
        "clap should reject {flag}; stderr was: {}",
        output.stderr,
    );
    Ok(())
}

#[tokio::test]
async fn graph_search_source_flag_is_removed() -> Result<()> {
    assert_flag_removed("--source", Some("i-aaaaaa")).await
}

#[tokio::test]
async fn graph_search_target_flag_is_removed() -> Result<()> {
    assert_flag_removed("--target", Some("i-aaaaaa")).await
}

#[tokio::test]
async fn graph_search_object_flag_is_removed() -> Result<()> {
    assert_flag_removed("--object", Some("i-aaaaaa")).await
}

#[tokio::test]
async fn graph_search_rel_type_flag_is_removed() -> Result<()> {
    assert_flag_removed("--rel-type", Some("child-of")).await
}

#[tokio::test]
async fn graph_search_transitive_flag_is_removed() -> Result<()> {
    assert_flag_removed("--transitive", None).await
}

#[tokio::test]
async fn graph_search_scope_flag_is_removed() -> Result<()> {
    assert_flag_removed("--scope", Some("i-aaaaaa")).await
}

#[tokio::test]
async fn graph_search_kind_flag_is_removed() -> Result<()> {
    assert_flag_removed("--kind", Some("patch")).await
}

#[tokio::test]
async fn relations_subcommand_is_removed() -> Result<()> {
    // PR 6 removed the legacy top-level `relations` CLI subcommand.
    // Invoking it must fail with clap's "unrecognized subcommand" error
    // (exit 2).
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let output = user.cli_expect_failure(&["relations", "list"]).await?;

    assert_eq!(output.status.code(), Some(2), "expected exit 2");
    let stderr_lower = output.stderr.to_lowercase();
    assert!(
        stderr_lower.contains("unrecognized subcommand")
            || stderr_lower.contains("unknown subcommand"),
        "expected clap unknown-subcommand error, got: {}",
        output.stderr,
    );
    Ok(())
}
