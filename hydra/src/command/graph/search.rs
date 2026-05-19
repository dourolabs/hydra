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
use hydra_common::graph::{ObjectKind, VerbosityLevel};
use hydra_common::HydraId;
use serde_json::Value;

use crate::client::HydraClientInterface;
use crate::command::graph::dispatch::{hydrate_by_id, kind_to_str, render_view, HydratedNode};
use crate::command::graph::selection::{self, SelectionFlags};
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

impl SearchParams {
    fn selection_flags(&self) -> SelectionFlags {
        SelectionFlags {
            source: self.source.clone(),
            target: self.target.clone(),
            object: self.object.clone(),
            rel_type: self.rel_type.clone(),
            transitive: self.transitive,
            scope: self.scope.clone(),
        }
    }
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
    let flags = params.selection_flags();
    if let Err(msg) = selection::validate(&flags) {
        eprintln!("error: {msg}");
        process::exit(2);
    }

    let node_ids = selection::resolve_node_ids(client, &flags).await?;
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

    #[test]
    fn selection_flags_mirrors_search_params() {
        let params = SearchParams {
            source: Some("i-aaaaaa".parse().unwrap()),
            target: Some("i-bbbbbb".parse().unwrap()),
            object: None,
            rel_type: Some("child-of".to_string()),
            transitive: true,
            scope: None,
            kinds: Vec::new(),
            verbosity: VerbosityLevel::L1,
            max_nodes: 10_000,
        };
        let flags = params.selection_flags();
        assert_eq!(flags.source, params.source);
        assert_eq!(flags.target, params.target);
        assert_eq!(flags.rel_type, params.rel_type);
        assert!(flags.transitive);
    }
}
