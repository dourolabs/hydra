//! `hydra graph search` — implementation.
//!
//! The selection input is the positional pipe-grammar query (parsed in
//! [`hydra_common::graph::query`]). The flow is:
//!
//! 1. Parse the query string. Parse errors print the caret block and exit 2.
//! 2. [`crate::command::graph::resolver::resolve`] walks the lowered query
//!    against the server, applying the inclusive-by-default contract per
//!    relation stage and the 3-call scope expansion per `scope` stage. The
//!    `kind=` stage is recorded as a post-hydration filter.
//! 3. Hydrate the terminal vertex set per-id.
//! 4. Apply the kind post-filter (if any) and render at `--verbosity`.

use std::collections::HashSet;
use std::io::Write;
use std::process;

use anyhow::{Context, Result};
use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};
use futures::FutureExt;
use hydra_common::graph::query::parse;
use hydra_common::graph::{ObjectKind, VerbosityLevel};
use hydra_common::HydraId;
use serde_json::Value;

use crate::client::HydraClientInterface;
use crate::command::graph::dispatch::{hydrate_by_id, HydratedNode};
use crate::command::graph::resolver::{resolve, Resolved};
use crate::command::graph::DEFAULT_HYDRATION_CONCURRENCY;
use crate::command::output::{CommandContext, ResolvedOutputFormat};
use crate::output_writer::write_stdout;

/// Inputs to [`run_search`] after CLI parsing.
pub struct SearchParams {
    pub query: String,
    pub verbosity: VerbosityLevel,
    pub max_nodes: usize,
}

/// Top-level entry point for `hydra graph search`.
///
/// User-input errors (parse error, node-budget cap exceeded) exit with code
/// 2; transport / server errors propagate as `anyhow::Error` (exit 1).
pub async fn run_search(
    client: &dyn HydraClientInterface,
    params: SearchParams,
    context: &CommandContext,
) -> Result<()> {
    let parsed = match parse(&params.query) {
        Ok(q) => q,
        Err(err) => {
            eprintln!("{err}");
            process::exit(2);
        }
    };

    let Resolved {
        node_ids,
        kind_filters,
    } = resolve(client, parsed.lower()).await?;

    if node_ids.len() > params.max_nodes {
        eprintln!(
            "error: matched node set ({}) exceeds --max-nodes ({}); narrow your selection (use --max-nodes to raise)",
            node_ids.len(),
            params.max_nodes,
        );
        process::exit(2);
    }

    let mut nodes = hydrate_all(client, node_ids).await?;
    apply_kind_filters(&mut nodes, &kind_filters);
    nodes.sort_by(|a, b| a.id().as_ref().cmp(b.id().as_ref()));

    let mut buffer = Vec::new();
    render(context.output_format, &nodes, params.verbosity, &mut buffer)?;
    write_stdout(&buffer)?;
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

/// Apply the resolver's recorded `kind=` post-filters to the hydrated set.
///
/// Each list comes from one `| kind=...` stage in the query; the set of
/// kinds allowed by the pipeline is their intersection. Empty `kind_filters`
/// (no kind stage in the query) is a no-op.
fn apply_kind_filters(nodes: &mut Vec<HydratedNode>, kind_filters: &[Vec<ObjectKind>]) {
    if kind_filters.is_empty() {
        return;
    }
    let mut iter = kind_filters.iter();
    let mut allowed: HashSet<ObjectKind> =
        iter.next().expect("non-empty").iter().copied().collect();
    for ks in iter {
        let next: HashSet<ObjectKind> = ks.iter().copied().collect();
        allowed = allowed.intersection(&next).copied().collect();
    }
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
        Value::String(node.kind().as_str().to_string()),
    );
    obj.insert(
        "id".to_string(),
        Value::String(node.id().as_ref().to_string()),
    );
    let view = node.render(level);
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
            let kind = node.kind().as_str();
            let view = node.render(level);
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
