//! `hydra graph search` — implementation.
//!
//! The selection input is the positional pipe-grammar query (parsed in
//! [`crate::command::graph::query`]). The flow is:
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
use hydra_common::graph::{ObjectKind, VerbosityLevel};
use hydra_common::HydraId;
use serde_json::Value;

use crate::client::HydraClientInterface;
use crate::command::graph::diff::write_view_fields;
use crate::command::graph::dispatch::{hydrate_by_id, HydratedNode};
use crate::command::graph::query::parse;
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
        let record = node.json_record(level);
        serde_json::to_writer(&mut *writer, &record)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

impl HydratedNode {
    fn json_record(&self, level: VerbosityLevel) -> Value {
        serde_json::json!({
            "id": self.id().as_ref(),
            "kind": self.kind().as_str(),
            "object": self.render(level),
        })
    }
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

    for (index, node) in nodes.iter().enumerate() {
        writeln!(writer, "{} {}", node.kind().as_str(), node.id().as_ref())?;
        let view = node.render(level);
        write_view_fields(writer, &view)?;
        if index + 1 < nodes.len() {
            writeln!(writer)?;
        }
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::status::make_status_def;
    use chrono::{DateTime, TimeZone, Utc};
    use hydra_common::api::v1::issues::{Issue, IssueStatus, IssueType, SessionSettings};
    use hydra_common::api::v1::patches::{Patch, PatchStatus, PatchVersionRecord};
    use hydra_common::issues::IssueVersionRecord;
    use hydra_common::users::Username;
    use hydra_common::{IssueId, PatchId, ProjectId};

    fn ts() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap()
    }

    fn sample_issue_node(id: &str, title: &str) -> HydratedNode {
        let issue_id: IssueId = id.parse().unwrap();
        let issue = Issue::new(
            IssueType::Task,
            title.to_string(),
            "long-form description body".to_string(),
            Username::from("creator"),
            String::new(),
            make_status_def(IssueStatus::Open.into()),
            ProjectId::default_project(),
            None,
            Some(SessionSettings::default()),
            Vec::new(),
            Vec::new(),
            false,
            None,
            None,
            None,
        );
        let record = IssueVersionRecord::new(issue_id, 1, ts(), issue, None, ts(), Vec::new());
        HydratedNode::Issue(Box::new(record))
    }

    fn sample_patch_node(id: &str, title: &str) -> HydratedNode {
        let patch_id: PatchId = id.parse().unwrap();
        let patch = Patch::new(
            title.to_string(),
            "patch description".to_string(),
            String::new(),
            PatchStatus::Open,
            false,
            Username::from("creator"),
            Vec::new(),
            "org/repo".parse().unwrap(),
            None,
            false,
            Some("feature/fix".to_string()),
            None,
            Some("main".to_string()),
        );
        let record = PatchVersionRecord::new(patch_id, 1, ts(), patch, None, ts(), Vec::new());
        HydratedNode::Patch(record)
    }

    #[test]
    fn render_jsonl_uses_envelope_with_id_kind_object() {
        let nodes = vec![sample_issue_node("i-aaaaaa", "an issue")];
        let mut buf = Vec::new();
        render_jsonl(&nodes, VerbosityLevel::L1, &mut buf).unwrap();
        let line = String::from_utf8(buf).unwrap();
        let value: Value = serde_json::from_str(line.trim()).expect("valid jsonl");
        assert_eq!(value["id"], "i-aaaaaa");
        assert_eq!(value["kind"], "issue");
        assert!(
            value["object"].is_object(),
            "object envelope should be an object: {value}",
        );
        assert_eq!(value["object"]["title"], "an issue");
        assert!(
            value.get("title").is_none(),
            "title must live under object, not at top level: {value}",
        );
        assert!(
            value["object"].get("id").is_none(),
            "id must not be duplicated inside object: {value}",
        );
        assert!(
            value["object"].get("kind").is_none(),
            "kind must not be duplicated inside object: {value}",
        );
    }

    #[test]
    fn render_jsonl_envelope_carries_verbosity_projection_at_l3() {
        let nodes = vec![sample_issue_node("i-aaaaaa", "an issue")];
        let mut buf = Vec::new();
        render_jsonl(&nodes, VerbosityLevel::L3, &mut buf).unwrap();
        let value: Value =
            serde_json::from_str(String::from_utf8(buf).unwrap().trim()).expect("valid jsonl");
        let object = &value["object"];
        assert!(
            object.get("description").is_some(),
            "L3 should include description under object: {value}",
        );
        assert!(
            object.get("creator").is_some(),
            "L3 should include creator under object: {value}",
        );
    }

    #[test]
    fn render_pretty_single_kind_l1_emits_per_record_block() {
        let nodes = vec![sample_issue_node("i-aaaaaa", "first issue")];
        let mut buf = Vec::new();
        render_pretty(&nodes, VerbosityLevel::L1, &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.starts_with("issue i-aaaaaa\n"), "got: {out}");
        assert!(out.contains("  title: \"first issue\""), "got: {out}");
        assert!(out.contains("  status: \"open\""), "got: {out}");
        assert!(
            !out.contains("description"),
            "L1 should not include description: {out}",
        );
        assert!(!out.contains("ID  "), "no table header expected: {out}");
    }

    #[test]
    fn render_pretty_single_kind_l3_includes_l3_only_fields() {
        let nodes = vec![sample_issue_node("i-aaaaaa", "first issue")];
        let mut buf = Vec::new();
        render_pretty(&nodes, VerbosityLevel::L3, &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("issue i-aaaaaa\n"), "got: {out}");
        assert!(
            out.contains("  description:"),
            "L3 missing description: {out}"
        );
        assert!(out.contains("  creator:"), "L3 missing creator: {out}");
    }

    #[test]
    fn render_pretty_mixed_kinds_l1_emits_blank_line_between_blocks() {
        let nodes = vec![
            sample_issue_node("i-aaaaaa", "issue title"),
            sample_patch_node("p-bbbbbb", "patch title"),
        ];
        let mut buf = Vec::new();
        render_pretty(&nodes, VerbosityLevel::L1, &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("issue i-aaaaaa\n"), "got: {out}");
        assert!(out.contains("patch p-bbbbbb\n"), "got: {out}");
        assert!(
            out.contains("  title: \"issue title\""),
            "issue title missing: {out}",
        );
        assert!(
            out.contains("  title: \"patch title\""),
            "patch title missing: {out}",
        );
        let issue_pos = out.find("issue i-aaaaaa").unwrap();
        let patch_pos = out.find("patch p-bbbbbb").unwrap();
        assert!(issue_pos < patch_pos, "ordering mismatch: {out}");
        let between = &out[issue_pos..patch_pos];
        assert!(
            between.contains("\n\n"),
            "expected blank line between records: {out}",
        );
        assert!(
            !out.ends_with("\n\n"),
            "trailing blank line should be omitted: {out:?}",
        );
    }

    #[test]
    fn render_pretty_empty_set_prints_placeholder() {
        let mut buf = Vec::new();
        render_pretty(&[], VerbosityLevel::L1, &mut buf).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "No nodes found.\n");
    }
}
