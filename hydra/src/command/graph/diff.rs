//! `hydra graph diff` — implementation.
//!
//! Selects nodes with the same flag surface as `search`, then for each node
//! renders the delta between its versions at `--since` and `--until`
//! projected through the kind's `view_lN`. Output is JSONL: `added` /
//! `removed` / `modified` records. The full algorithm lives client-side; no
//! server route is involved.

use std::collections::{BTreeSet, HashSet};
use std::io::{self, Write};
use std::process;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};
use futures::FutureExt;
use hydra_common::graph::{GraphView, ObjectKind, VerbosityLevel};
use hydra_common::time::{parse_window_arg_with_now, TimeParseError};
use hydra_common::versioning::{VersionNumber, Versioned};
use hydra_common::HydraId;
use serde_json::{Map, Value};

use crate::client::HydraClientInterface;
use crate::command::graph::dispatch::{fetch_versions, kind_to_str, VersionedNode};
use crate::command::graph::selection::{self, SelectionFlags};
use crate::command::graph::{KindArg, DEFAULT_HYDRATION_CONCURRENCY};
use crate::command::output::{CommandContext, ResolvedOutputFormat};

/// Selection and rendering inputs for `hydra graph diff`.
#[derive(Debug, Clone)]
pub struct DiffParams {
    pub since: String,
    pub until: String,
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

impl DiffParams {
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

/// One row of the diff output.
#[derive(Debug, Clone)]
enum DiffRecord {
    Added {
        kind: ObjectKind,
        id: HydraId,
        version_to: VersionNumber,
        object: Value,
    },
    Removed {
        kind: ObjectKind,
        id: HydraId,
        version_from: VersionNumber,
        object: Option<Value>,
    },
    Modified {
        kind: ObjectKind,
        id: HydraId,
        version_from: VersionNumber,
        version_to: VersionNumber,
        fields: Map<String, Value>,
    },
}

impl DiffRecord {
    fn id(&self) -> &HydraId {
        match self {
            DiffRecord::Added { id, .. }
            | DiffRecord::Removed { id, .. }
            | DiffRecord::Modified { id, .. } => id,
        }
    }
}

/// Top-level entry point for `hydra graph diff`.
///
/// User-input errors (mutually-exclusive flags, empty selection, time-parse
/// errors, node-budget cap exceeded) exit with code 2; transport / server
/// errors propagate as `anyhow::Error` (exit 1).
pub async fn run_diff(
    client: &dyn HydraClientInterface,
    params: DiffParams,
    context: &CommandContext,
) -> Result<()> {
    let flags = params.selection_flags();
    if let Err(msg) = selection::validate(&flags) {
        eprintln!("error: {msg}");
        process::exit(2);
    }

    let now = Utc::now();
    let since = match parse_window(&params.since, "since", now) {
        Ok(ts) => ts,
        Err(msg) => {
            eprintln!("error: {msg}");
            process::exit(2);
        }
    };
    let until = match parse_window(&params.until, "until", now) {
        Ok(ts) => ts,
        Err(msg) => {
            eprintln!("error: {msg}");
            process::exit(2);
        }
    };
    if since > until {
        eprintln!("error: --since ({since}) must be at or before --until ({until})");
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

    let filtered_ids = filter_by_kind(node_ids, &params.kinds);
    let mut records = compute_diff_records(client, filtered_ids, since, until, params.verbosity)
        .await
        .context("failed to compute diff records")?;
    records.sort_by(|a, b| a.id().as_ref().cmp(b.id().as_ref()));

    let mut stdout = io::stdout().lock();
    render(context.output_format, &records, &mut stdout)?;
    Ok(())
}

fn parse_window(raw: &str, flag: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>, String> {
    parse_window_arg_with_now(raw, now).map_err(|e| match e {
        TimeParseError::Empty => format!("--{flag} value is empty"),
        other => format!("--{flag}: {other}"),
    })
}

fn filter_by_kind(ids: Vec<HydraId>, kinds: &[KindArg]) -> Vec<HydraId> {
    if kinds.is_empty() {
        return ids;
    }
    let allowed: HashSet<ObjectKind> = kinds.iter().map(|k| k.as_object_kind()).collect();
    ids.into_iter()
        .filter(|id| id_kind(id).map(|k| allowed.contains(&k)).unwrap_or(false))
        .collect()
}

fn id_kind(id: &HydraId) -> Option<ObjectKind> {
    if id.as_issue_id().is_some() {
        Some(ObjectKind::Issue)
    } else if id.as_patch_id().is_some() {
        Some(ObjectKind::Patch)
    } else if id.as_document_id().is_some() {
        Some(ObjectKind::Document)
    } else if id.as_conversation_id().is_some() {
        Some(ObjectKind::Conversation)
    } else {
        None
    }
}

/// Fetch versions for each id (bounded concurrency) and convert into diff
/// records. Empty / unchanged nodes contribute nothing to the output.
async fn compute_diff_records(
    client: &dyn HydraClientInterface,
    ids: Vec<HydraId>,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    level: VerbosityLevel,
) -> Result<Vec<DiffRecord>> {
    let total = ids.len();
    let mut iter = ids.into_iter();
    let mut in_flight: FuturesUnordered<BoxFuture<'_, Result<(HydraId, VersionedNode)>>> =
        FuturesUnordered::new();
    let mut out: Vec<DiffRecord> = Vec::with_capacity(total);

    for _ in 0..DEFAULT_HYDRATION_CONCURRENCY {
        if let Some(id) = iter.next() {
            in_flight.push(fetch_for_id(client, id).boxed());
        } else {
            break;
        }
    }

    while let Some(result) = in_flight.next().await {
        let (id, versioned) = result?;
        if let Some(record) = classify_versioned(&id, versioned, since, until, level) {
            out.push(record);
        }
        if let Some(id) = iter.next() {
            in_flight.push(fetch_for_id(client, id).boxed());
        }
    }
    Ok(out)
}

async fn fetch_for_id(
    client: &dyn HydraClientInterface,
    id: HydraId,
) -> Result<(HydraId, VersionedNode)> {
    let versioned = fetch_versions(client, &id).await?;
    Ok((id, versioned))
}

fn classify_versioned(
    id: &HydraId,
    versioned: VersionedNode,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    level: VerbosityLevel,
) -> Option<DiffRecord> {
    let kind = versioned.kind();
    match versioned {
        VersionedNode::Issue(versions) => {
            classify_versions(id, kind, &versions, since, until, level)
        }
        VersionedNode::Patch(versions) => {
            classify_versions(id, kind, &versions, since, until, level)
        }
        VersionedNode::Document(versions) => {
            classify_versions(id, kind, &versions, since, until, level)
        }
        VersionedNode::Conversation(versions) => {
            classify_versions(id, kind, &versions, since, until, level)
        }
    }
}

/// Classify a per-node version sequence into a diff record (or `None` if
/// unchanged / empty).
fn classify_versions<T>(
    id: &HydraId,
    kind: ObjectKind,
    versions: &[Versioned<T>],
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    level: VerbosityLevel,
) -> Option<DiffRecord>
where
    T: GraphView,
{
    let v_start = latest_at_or_before(versions, since);
    let v_end = latest_at_or_before(versions, until);
    match (v_start, v_end) {
        (None, None) => None,
        (None, Some(end)) => Some(DiffRecord::Added {
            kind,
            id: id.clone(),
            version_to: end.version,
            object: view_at(&end.item, level),
        }),
        (Some(start), None) => {
            let object = match level {
                VerbosityLevel::L1 => None,
                _ => Some(view_at(&start.item, level)),
            };
            Some(DiffRecord::Removed {
                kind,
                id: id.clone(),
                version_from: start.version,
                object,
            })
        }
        (Some(start), Some(end)) => {
            let before = view_at(&start.item, level);
            let after = view_at(&end.item, level);
            if before == after {
                return None;
            }
            let mut fields = Map::new();
            diff_into(&mut fields, "", &before, &after);
            if fields.is_empty() {
                return None;
            }
            Some(DiffRecord::Modified {
                kind,
                id: id.clone(),
                version_from: start.version,
                version_to: end.version,
                fields,
            })
        }
    }
}

fn view_at<T: GraphView>(item: &T, level: VerbosityLevel) -> Value {
    match level {
        VerbosityLevel::L1 => item.view_l1(),
        VerbosityLevel::L2 => item.view_l2(),
        VerbosityLevel::L3 => item.view_l3(),
    }
}

/// Return the latest version whose `timestamp <= ts`, scanning the sequence
/// (which is in increasing timestamp order in practice but not assumed).
fn latest_at_or_before<T>(versions: &[Versioned<T>], ts: DateTime<Utc>) -> Option<&Versioned<T>> {
    versions
        .iter()
        .filter(|v| v.timestamp <= ts)
        .max_by_key(|v| v.timestamp)
}

/// Flatten a JSON-value diff into `{ "<dotted.path>": { "before": x, "after": y } }`.
fn diff_into(out: &mut Map<String, Value>, prefix: &str, before: &Value, after: &Value) {
    match (before, after) {
        (Value::Object(b), Value::Object(a)) => {
            let mut keys: BTreeSet<String> = BTreeSet::new();
            for k in b.keys() {
                keys.insert(k.clone());
            }
            for k in a.keys() {
                keys.insert(k.clone());
            }
            for k in keys {
                let bv = b.get(&k).cloned().unwrap_or(Value::Null);
                let av = a.get(&k).cloned().unwrap_or(Value::Null);
                let path = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                diff_into(out, &path, &bv, &av);
            }
        }
        (Value::Array(b), Value::Array(a)) => {
            let max = b.len().max(a.len());
            for i in 0..max {
                let bv = b.get(i).cloned().unwrap_or(Value::Null);
                let av = a.get(i).cloned().unwrap_or(Value::Null);
                let path = if prefix.is_empty() {
                    i.to_string()
                } else {
                    format!("{prefix}.{i}")
                };
                diff_into(out, &path, &bv, &av);
            }
        }
        (b, a) if b == a => { /* unchanged */ }
        (b, a) => {
            out.insert(
                prefix.to_string(),
                serde_json::json!({ "before": b, "after": a }),
            );
        }
    }
}

fn render(
    format: ResolvedOutputFormat,
    records: &[DiffRecord],
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => render_jsonl(records, writer),
        ResolvedOutputFormat::Pretty => render_pretty(records, writer),
    }
}

fn render_jsonl(records: &[DiffRecord], writer: &mut impl Write) -> Result<()> {
    for record in records {
        let value = record_to_jsonl(record);
        serde_json::to_writer(&mut *writer, &value)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn record_to_jsonl(record: &DiffRecord) -> Value {
    match record {
        DiffRecord::Added {
            kind,
            id,
            version_to,
            object,
        } => serde_json::json!({
            "change": "added",
            "kind": kind_to_str(*kind),
            "id": id.as_ref(),
            "version": { "to": version_to },
            "object": object,
        }),
        DiffRecord::Removed {
            kind,
            id,
            version_from,
            object,
        } => {
            let mut obj = serde_json::Map::new();
            obj.insert("change".to_string(), Value::String("removed".to_string()));
            obj.insert(
                "kind".to_string(),
                Value::String(kind_to_str(*kind).to_string()),
            );
            obj.insert("id".to_string(), Value::String(id.as_ref().to_string()));
            obj.insert(
                "version".to_string(),
                serde_json::json!({ "from": version_from, "to": Value::Null }),
            );
            if let Some(view) = object {
                obj.insert("object".to_string(), view.clone());
            }
            Value::Object(obj)
        }
        DiffRecord::Modified {
            kind,
            id,
            version_from,
            version_to,
            fields,
        } => serde_json::json!({
            "change": "modified",
            "kind": kind_to_str(*kind),
            "id": id.as_ref(),
            "version": { "from": version_from, "to": version_to },
            "fields": Value::Object(fields.clone()),
        }),
    }
}

fn render_pretty(records: &[DiffRecord], writer: &mut impl Write) -> Result<()> {
    if records.is_empty() {
        writeln!(writer, "No changes.")?;
        writer.flush()?;
        return Ok(());
    }
    for record in records {
        match record {
            DiffRecord::Added {
                kind,
                id,
                version_to,
                object,
            } => {
                writeln!(
                    writer,
                    "{kind} {id} (added, version {version_to})",
                    kind = kind_to_str(*kind),
                    id = id.as_ref(),
                )?;
                writeln!(writer, "  + NEW")?;
                write_object_block(writer, object)?;
            }
            DiffRecord::Removed {
                kind,
                id,
                version_from,
                object,
            } => {
                writeln!(
                    writer,
                    "{kind} {id} (removed, last version {version_from})",
                    kind = kind_to_str(*kind),
                    id = id.as_ref(),
                )?;
                writeln!(writer, "  - REMOVED")?;
                if let Some(view) = object {
                    write_object_block(writer, view)?;
                }
            }
            DiffRecord::Modified {
                kind,
                id,
                version_from,
                version_to,
                fields,
            } => {
                writeln!(
                    writer,
                    "{kind} {id} (modified, version {version_from} → {version_to})",
                    kind = kind_to_str(*kind),
                    id = id.as_ref(),
                )?;
                for (path, change) in fields {
                    let before = change
                        .get("before")
                        .map(format_value_inline)
                        .unwrap_or_else(|| "null".to_string());
                    let after = change
                        .get("after")
                        .map(format_value_inline)
                        .unwrap_or_else(|| "null".to_string());
                    writeln!(writer, "  + {path}: {before} → {after}")?;
                }
            }
        }
    }
    writer.flush()?;
    Ok(())
}

fn write_object_block(writer: &mut impl Write, object: &Value) -> Result<()> {
    if let Value::Object(map) = object {
        for (k, v) in map {
            writeln!(writer, "    {k}: {value}", value = format_value_inline(v))?;
        }
    } else {
        writeln!(writer, "    {value}", value = format_value_inline(object))?;
    }
    Ok(())
}

fn format_value_inline(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    fn v<T>(item: T, version: VersionNumber, secs: i64) -> Versioned<T> {
        Versioned::new(item, version, ts(secs), ts(secs))
    }

    #[test]
    fn latest_at_or_before_returns_none_when_all_after() {
        let versions = vec![v((), 1, 100), v((), 2, 200)];
        assert!(latest_at_or_before(&versions, ts(50)).is_none());
    }

    #[test]
    fn latest_at_or_before_returns_latest_within_bound() {
        let versions = vec![v((), 1, 100), v((), 2, 200), v((), 3, 300)];
        let got = latest_at_or_before(&versions, ts(250)).unwrap();
        assert_eq!(got.version, 2);
    }

    #[test]
    fn latest_at_or_before_accepts_equal_timestamp() {
        let versions = vec![v((), 1, 100), v((), 2, 200)];
        let got = latest_at_or_before(&versions, ts(200)).unwrap();
        assert_eq!(got.version, 2);
    }

    #[test]
    fn diff_into_empty_for_equal_values() {
        let before = json!({ "status": "open", "title": "x" });
        let after = json!({ "status": "open", "title": "x" });
        let mut out = Map::new();
        diff_into(&mut out, "", &before, &after);
        assert!(out.is_empty());
    }

    #[test]
    fn diff_into_flat_scalar_change() {
        let before = json!({ "status": "open" });
        let after = json!({ "status": "in_progress" });
        let mut out = Map::new();
        diff_into(&mut out, "", &before, &after);
        assert_eq!(
            out.get("status"),
            Some(&json!({ "before": "open", "after": "in_progress" }))
        );
    }

    #[test]
    fn diff_into_nested_object_uses_dotted_path() {
        let before = json!({ "a": { "b": 1 } });
        let after = json!({ "a": { "b": 2 } });
        let mut out = Map::new();
        diff_into(&mut out, "", &before, &after);
        assert_eq!(out.get("a.b"), Some(&json!({ "before": 1, "after": 2 })));
    }

    #[test]
    fn diff_into_array_uses_indexed_path() {
        let before = json!({ "deps": [{ "type": "child" }] });
        let after = json!({ "deps": [{ "type": "blocked" }] });
        let mut out = Map::new();
        diff_into(&mut out, "", &before, &after);
        assert_eq!(
            out.get("deps.0.type"),
            Some(&json!({ "before": "child", "after": "blocked" }))
        );
    }

    #[test]
    fn diff_into_missing_key_treated_as_null_to_value() {
        let before = json!({});
        let after = json!({ "title": "hi" });
        let mut out = Map::new();
        diff_into(&mut out, "", &before, &after);
        assert_eq!(
            out.get("title"),
            Some(&json!({ "before": Value::Null, "after": "hi" }))
        );
    }

    #[test]
    fn id_kind_dispatches_by_prefix() {
        assert_eq!(
            id_kind(&"i-aaaaaa".parse().unwrap()),
            Some(ObjectKind::Issue)
        );
        assert_eq!(
            id_kind(&"p-bbbbbb".parse().unwrap()),
            Some(ObjectKind::Patch)
        );
        assert_eq!(
            id_kind(&"d-cccccc".parse().unwrap()),
            Some(ObjectKind::Document)
        );
    }

    #[test]
    fn filter_by_kind_empty_kinds_returns_input_unchanged() {
        let ids: Vec<HydraId> = vec!["i-aaaaaa".parse().unwrap(), "p-bbbbbb".parse().unwrap()];
        let filtered = filter_by_kind(ids.clone(), &[]);
        assert_eq!(filtered, ids);
    }

    #[test]
    fn filter_by_kind_drops_other_kinds() {
        let ids: Vec<HydraId> = vec!["i-aaaaaa".parse().unwrap(), "p-bbbbbb".parse().unwrap()];
        let filtered = filter_by_kind(ids, &[KindArg::Issue]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].as_ref(), "i-aaaaaa");
    }

    // A minimal GraphView impl used to exercise the classify_versions branches
    // without pulling in a full Issue / Patch / Document fixture.
    #[derive(Clone, Debug)]
    struct StubItem {
        title: String,
        status: String,
    }

    impl GraphView for StubItem {
        const KIND: ObjectKind = ObjectKind::Issue;
        fn view_l1(&self) -> Value {
            json!({ "title": self.title, "status": self.status })
        }
        fn view_l2(&self) -> Value {
            json!({ "title": self.title, "status": self.status })
        }
        fn view_l3(&self) -> Value {
            json!({ "title": self.title, "status": self.status })
        }
    }

    fn stub(title: &str, status: &str, version: u64, secs: i64) -> Versioned<StubItem> {
        v(
            StubItem {
                title: title.to_string(),
                status: status.to_string(),
            },
            version,
            secs,
        )
    }

    #[test]
    fn classify_versions_emits_modified_when_view_differs() {
        let id: HydraId = "i-aaaaaa".parse().unwrap();
        let versions = vec![stub("t", "open", 1, 100), stub("t", "in-progress", 2, 200)];
        let record = classify_versions(
            &id,
            ObjectKind::Issue,
            &versions,
            ts(150),
            ts(250),
            VerbosityLevel::L1,
        )
        .expect("modified record");
        match record {
            DiffRecord::Modified {
                version_from,
                version_to,
                fields,
                ..
            } => {
                assert_eq!(version_from, 1);
                assert_eq!(version_to, 2);
                assert_eq!(
                    fields.get("status"),
                    Some(&json!({ "before": "open", "after": "in-progress" }))
                );
            }
            other => panic!("expected Modified, got {other:?}"),
        }
    }

    #[test]
    fn classify_versions_emits_added_when_no_version_before_since() {
        let id: HydraId = "i-aaaaaa".parse().unwrap();
        let versions = vec![stub("t", "open", 1, 200)];
        let record = classify_versions(
            &id,
            ObjectKind::Issue,
            &versions,
            ts(100),
            ts(300),
            VerbosityLevel::L1,
        )
        .expect("added record");
        match record {
            DiffRecord::Added {
                version_to, object, ..
            } => {
                assert_eq!(version_to, 1);
                assert_eq!(object, json!({ "title": "t", "status": "open" }),);
            }
            other => panic!("expected Added, got {other:?}"),
        }
    }

    #[test]
    fn classify_versions_emits_removed_when_no_version_at_or_before_until() {
        // Construct a sequence where the only version is *after* `until`, so
        // we cover the (Some, None) branch even though no real-world delete
        // surfaces it this way.
        let id: HydraId = "i-aaaaaa".parse().unwrap();
        let versions = vec![stub("t", "open", 7, 100)];
        // since = 200 picks up v1 (timestamp 100). until = 50 picks up nothing.
        let record = classify_versions(
            &id,
            ObjectKind::Issue,
            &versions,
            ts(200),
            ts(50),
            VerbosityLevel::L2,
        )
        .expect("removed record");
        match record {
            DiffRecord::Removed {
                version_from,
                object,
                ..
            } => {
                assert_eq!(version_from, 7);
                assert!(object.is_some(), "L2 removed should include object");
            }
            other => panic!("expected Removed, got {other:?}"),
        }
    }

    #[test]
    fn classify_versions_removed_at_l1_omits_object() {
        let id: HydraId = "i-aaaaaa".parse().unwrap();
        let versions = vec![stub("t", "open", 1, 100)];
        let record = classify_versions(
            &id,
            ObjectKind::Issue,
            &versions,
            ts(200),
            ts(50),
            VerbosityLevel::L1,
        )
        .expect("removed record");
        match record {
            DiffRecord::Removed { object, .. } => {
                assert!(object.is_none(), "L1 removed must omit object");
            }
            other => panic!("expected Removed, got {other:?}"),
        }
    }

    #[test]
    fn classify_versions_returns_none_when_unchanged() {
        let id: HydraId = "i-aaaaaa".parse().unwrap();
        let versions = vec![stub("t", "open", 1, 100), stub("t", "open", 2, 200)];
        let record = classify_versions(
            &id,
            ObjectKind::Issue,
            &versions,
            ts(150),
            ts(250),
            VerbosityLevel::L1,
        );
        assert!(record.is_none(), "expected unchanged: {record:?}");
    }
}
