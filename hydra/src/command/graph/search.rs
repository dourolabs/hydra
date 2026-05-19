//! `hydra graph search` — implementation.
//!
//! Selection: same flags as `hydra relations list` plus `--scope <ID>`. The
//! result is the set of **nodes** addressed by the matching edges, hydrated
//! and projected through the per-kind `GraphView::view_lN` impl.

use std::collections::HashSet;
use std::io::{self, Write};
use std::process;

use anyhow::{Context, Result};
use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};
use futures::FutureExt;
use hydra_common::api::v1::relations::{ListRelationsRequest, ListRelationsResponse};
use hydra_common::graph::{ObjectKind, VerbosityLevel};
use hydra_common::HydraId;
use serde_json::Value;

use crate::client::HydraClientInterface;
use crate::command::graph::dispatch::{hydrate_by_id, kind_to_str, render_view, HydratedNode};
use crate::command::graph::{KindArg, DEFAULT_HYDRATION_CONCURRENCY};
use crate::command::output::{CommandContext, ResolvedOutputFormat};

/// Selection and rendering inputs for `hydra graph search`.
#[derive(Debug, Clone)]
pub struct SearchParams {
    pub source: Option<HydraId>,
    pub target: Option<HydraId>,
    pub object: Option<HydraId>,
    pub rel_type: Option<String>,
    pub transitive: bool,
    pub scope: Option<HydraId>,
    pub kinds: Vec<KindArg>,
    pub verbosity: VerbosityLevel,
    pub max_nodes: usize,
}

/// Top-level entry point for `hydra graph search`.
///
/// User-input errors (mutually-exclusive flags, empty selection, node-budget
/// cap exceeded) exit with code 2; transport / server errors propagate as
/// `anyhow::Error` (exit 1).
pub async fn run_search(
    client: &dyn HydraClientInterface,
    params: SearchParams,
    context: &CommandContext,
) -> Result<()> {
    if let Err(msg) = validate(&params) {
        eprintln!("error: {msg}");
        process::exit(2);
    }

    let node_ids = resolve_node_ids(client, &params).await?;
    if node_ids.len() > params.max_nodes {
        eprintln!(
            "error: matched node set ({}) exceeds --max-nodes ({}); narrow your selection (use --max-nodes to raise)",
            node_ids.len(),
            params.max_nodes,
        );
        process::exit(2);
    }

    let mut nodes = hydrate_all(client, node_ids).await?;
    apply_kind_filter(&mut nodes, &params.kinds);
    nodes.sort_by(|a, b| a.id().as_ref().cmp(b.id().as_ref()));

    let mut stdout = io::stdout().lock();
    render(context.output_format, &nodes, params.verbosity, &mut stdout)?;
    Ok(())
}

/// Validate the CLI flag combinations. Returns an error message on misuse.
pub(crate) fn validate(params: &SearchParams) -> Result<(), String> {
    if params.scope.is_some()
        && (params.source.is_some() || params.target.is_some() || params.object.is_some())
    {
        return Err("--scope is mutually exclusive with --source/--target/--object".to_string());
    }
    if params.scope.is_none()
        && params.source.is_none()
        && params.target.is_none()
        && params.object.is_none()
    {
        return Err(
            "at least one of --source, --target, --object, or --scope is required".to_string(),
        );
    }
    Ok(())
}

/// Step 1 of the algorithm: resolve the set of node ids to hydrate.
pub(crate) async fn resolve_node_ids(
    client: &dyn HydraClientInterface,
    params: &SearchParams,
) -> Result<Vec<HydraId>> {
    if let Some(scope) = &params.scope {
        resolve_scope_node_ids(client, scope).await
    } else {
        let query = ListRelationsRequest {
            source_id: params.source.clone(),
            source_ids: None,
            target_id: params.target.clone(),
            target_ids: None,
            object_id: params.object.clone(),
            rel_type: params.rel_type.clone(),
            transitive: if params.transitive { Some(true) } else { None },
        };
        let response = client
            .list_relations(&query)
            .await
            .context("failed to list relations")?;
        Ok(node_ids_from_edges(&response))
    }
}

/// Union of `source_id` and `target_id` across each returned edge.
fn node_ids_from_edges(response: &ListRelationsResponse) -> Vec<HydraId> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<HydraId> = Vec::new();
    for edge in &response.relations {
        if seen.insert(edge.source_id.as_ref().to_string()) {
            out.push(edge.source_id.clone());
        }
        if seen.insert(edge.target_id.as_ref().to_string()) {
            out.push(edge.target_id.clone());
        }
    }
    out
}

/// Resolve `--scope <ID>` to the full node set per the design doc:
/// {scope} ∪ descendants(child-of, transitive) ∪ has-patch targets ∪ has-document targets.
/// `refers-to` is intentionally **not** fanned out.
async fn resolve_scope_node_ids(
    client: &dyn HydraClientInterface,
    scope: &HydraId,
) -> Result<Vec<HydraId>> {
    // 1. Descendants via child-of (transitive).
    let descendants_query = ListRelationsRequest {
        target_id: Some(scope.clone()),
        rel_type: Some("child-of".to_string()),
        transitive: Some(true),
        ..Default::default()
    };
    let descendants_response = client
        .list_relations(&descendants_query)
        .await
        .context("failed to list child-of descendants for --scope")?;

    // For child-of edges with target=scope (transitive), each edge's source is
    // a descendant issue.
    let mut s_set: Vec<HydraId> = vec![scope.clone()];
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(scope.as_ref().to_string());
    for edge in &descendants_response.relations {
        if seen.insert(edge.source_id.as_ref().to_string()) {
            s_set.push(edge.source_id.clone());
        }
    }

    // 2. In parallel, for each of has-patch and has-document, fetch targets
    // whose source is in `s_set`.
    let source_ids_csv = s_set
        .iter()
        .map(|id| id.as_ref().to_string())
        .collect::<Vec<_>>()
        .join(",");

    let mut futures = FuturesUnordered::new();
    for rel in ["has-patch", "has-document"] {
        let query = ListRelationsRequest {
            source_ids: Some(source_ids_csv.clone()),
            rel_type: Some(rel.to_string()),
            ..Default::default()
        };
        futures.push(async move { client.list_relations(&query).await });
    }

    let mut all_ids = s_set;
    while let Some(result) = futures.next().await {
        let response =
            result.context("failed to list has-patch/has-document targets for --scope")?;
        for edge in &response.relations {
            if seen.insert(edge.target_id.as_ref().to_string()) {
                all_ids.push(edge.target_id.clone());
            }
        }
    }
    Ok(all_ids)
}

/// Hydrate each id concurrently (bounded by `DEFAULT_HYDRATION_CONCURRENCY`).
async fn hydrate_all(
    client: &dyn HydraClientInterface,
    ids: Vec<HydraId>,
) -> Result<Vec<HydratedNode>> {
    let total = ids.len();
    let mut iter = ids.into_iter();
    let mut in_flight: FuturesUnordered<BoxFuture<'_, Result<HydratedNode>>> =
        FuturesUnordered::new();
    let mut nodes = Vec::with_capacity(total);

    for _ in 0..DEFAULT_HYDRATION_CONCURRENCY {
        if let Some(id) = iter.next() {
            in_flight.push(async move { hydrate_by_id(client, &id).await }.boxed());
        } else {
            break;
        }
    }

    while let Some(result) = in_flight.next().await {
        nodes.push(result.context("failed to hydrate graph node")?);
        if let Some(id) = iter.next() {
            in_flight.push(async move { hydrate_by_id(client, &id).await }.boxed());
        }
    }
    Ok(nodes)
}

fn apply_kind_filter(nodes: &mut Vec<HydratedNode>, kinds: &[KindArg]) {
    if kinds.is_empty() {
        return;
    }
    let allowed: HashSet<ObjectKind> = kinds.iter().map(|k| k.as_object_kind()).collect();
    nodes.retain(|n| allowed.contains(&n.kind()));
}

fn render(
    format: ResolvedOutputFormat,
    nodes: &[HydratedNode],
    level: VerbosityLevel,
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => render_jsonl(nodes, level, writer),
        ResolvedOutputFormat::Pretty => render_pretty(nodes, level, writer),
    }
}

fn render_jsonl(
    nodes: &[HydratedNode],
    level: VerbosityLevel,
    writer: &mut impl Write,
) -> Result<()> {
    for node in nodes {
        let record = json_record(node, level);
        serde_json::to_writer(&mut *writer, &record)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn json_record(node: &HydratedNode, level: VerbosityLevel) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert(
        "kind".to_string(),
        Value::String(node.kind_str().to_string()),
    );
    obj.insert(
        "id".to_string(),
        Value::String(node.id().as_ref().to_string()),
    );
    let view = render_view(node, level);
    if let Value::Object(fields) = view {
        for (k, v) in fields {
            obj.insert(k, v);
        }
    } else {
        // view_lN returns an object today; if a kind ever returns a non-object
        // (e.g. an array), preserve it under a "view" key so callers still get
        // structured data.
        obj.insert("view".to_string(), view);
    }
    Value::Object(obj)
}

fn render_pretty(
    nodes: &[HydratedNode],
    level: VerbosityLevel,
    writer: &mut impl Write,
) -> Result<()> {
    if nodes.is_empty() {
        writeln!(writer, "No nodes found.")?;
        writer.flush()?;
        return Ok(());
    }

    let rows: Vec<(String, &'static str, String, String)> = nodes
        .iter()
        .map(|node| {
            let id = node.id().as_ref().to_string();
            let kind = kind_to_str(node.kind());
            let view = render_view(node, level);
            let title = view
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let status = view
                .get("status")
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_else(|| {
                    // Documents have no status — fall back to path if present.
                    view.get("path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string()
                });
            (id, kind, title, status)
        })
        .collect();

    let id_w = rows.iter().map(|r| r.0.len()).max().unwrap_or(2).max(2);
    let kind_w = rows.iter().map(|r| r.1.len()).max().unwrap_or(4).max(4);
    let title_w = rows.iter().map(|r| r.2.len()).max().unwrap_or(5).max(5);

    writeln!(
        writer,
        "{:<id_w$}  {:<kind_w$}  {:<title_w$}  STATUS",
        "ID", "KIND", "TITLE",
    )?;
    writeln!(
        writer,
        "{:<id_w$}  {:<kind_w$}  {:<title_w$}  {}",
        "-".repeat(id_w),
        "-".repeat(kind_w),
        "-".repeat(title_w),
        "-".repeat(6),
    )?;
    for (id, kind, title, status) in &rows {
        writeln!(
            writer,
            "{id:<id_w$}  {kind:<kind_w$}  {title:<title_w$}  {status}"
        )?;
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_params() -> SearchParams {
        SearchParams {
            source: None,
            target: None,
            object: None,
            rel_type: None,
            transitive: false,
            scope: None,
            kinds: Vec::new(),
            verbosity: VerbosityLevel::L1,
            max_nodes: 10_000,
        }
    }

    #[test]
    fn validate_rejects_empty_selection() {
        let err = validate(&empty_params()).unwrap_err();
        assert!(err.contains("at least one of"), "got: {err}");
    }

    #[test]
    fn validate_rejects_scope_with_source() {
        let params = SearchParams {
            scope: Some("i-aaaaaa".parse().unwrap()),
            source: Some("i-bbbbbb".parse().unwrap()),
            ..empty_params()
        };
        let err = validate(&params).unwrap_err();
        assert!(err.contains("mutually exclusive"), "got: {err}");
    }

    #[test]
    fn validate_rejects_scope_with_target() {
        let params = SearchParams {
            scope: Some("i-aaaaaa".parse().unwrap()),
            target: Some("i-bbbbbb".parse().unwrap()),
            ..empty_params()
        };
        let err = validate(&params).unwrap_err();
        assert!(err.contains("mutually exclusive"), "got: {err}");
    }

    #[test]
    fn validate_rejects_scope_with_object() {
        let params = SearchParams {
            scope: Some("i-aaaaaa".parse().unwrap()),
            object: Some("i-bbbbbb".parse().unwrap()),
            ..empty_params()
        };
        let err = validate(&params).unwrap_err();
        assert!(err.contains("mutually exclusive"), "got: {err}");
    }

    #[test]
    fn validate_accepts_object_only() {
        let params = SearchParams {
            object: Some("i-aaaaaa".parse().unwrap()),
            ..empty_params()
        };
        assert!(validate(&params).is_ok());
    }

    #[test]
    fn validate_accepts_scope_alone() {
        let params = SearchParams {
            scope: Some("i-aaaaaa".parse().unwrap()),
            ..empty_params()
        };
        assert!(validate(&params).is_ok());
    }

    #[test]
    fn node_ids_from_edges_dedupes_and_preserves_order() {
        use hydra_common::api::v1::relations::RelationResponse;
        let response = ListRelationsResponse {
            relations: vec![
                RelationResponse {
                    source_id: "i-aaaaaa".parse().unwrap(),
                    target_id: "i-bbbbbb".parse().unwrap(),
                    rel_type: "child-of".to_string(),
                },
                RelationResponse {
                    source_id: "i-aaaaaa".parse().unwrap(),
                    target_id: "p-cccccc".parse().unwrap(),
                    rel_type: "has-patch".to_string(),
                },
            ],
        };
        let ids = node_ids_from_edges(&response);
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0].as_ref(), "i-aaaaaa");
        assert_eq!(ids[1].as_ref(), "i-bbbbbb");
        assert_eq!(ids[2].as_ref(), "p-cccccc");
    }
}
