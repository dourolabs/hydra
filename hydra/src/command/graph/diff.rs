//! `hydra graph diff` — implementation.
//!
//! Selects nodes via the pipe-DSL resolver (shared with `search`/`log`), then
//! for each node renders the delta between its versions at `--since` and
//! `--until` projected through the kind's `view_lN`. Output is JSONL:
//! `added` / `removed` / `modified` records.

use std::collections::BTreeMap;
use std::io::Write;
use std::process;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};
use futures::FutureExt;
use hydra_common::graph::{ObjectKind, VerbosityLevel};
use hydra_common::time::HydraTime;
use hydra_common::versioning::VersionNumber;
use hydra_common::HydraId;
use serde_json::Value;

use crate::client::HydraClientInterface;
use crate::command::graph::dispatch::{fetch_versions, VersionView, VersionedNode};
use crate::command::graph::query::parse as parse_query;
use crate::command::graph::resolver::{resolve, Resolved};
use crate::command::graph::DEFAULT_HYDRATION_CONCURRENCY;
use crate::command::output::{CommandContext, ResolvedOutputFormat};
use crate::output_writer::write_stdout;

/// Selection and rendering inputs for `hydra graph diff`.
#[derive(Debug, Clone)]
pub struct DiffParams {
    /// Pipe-grammar query string. Parsed at the top of [`run_diff`]; a parse
    /// error exits with code 2 and the caret-quoted message.
    pub query: String,
    pub since: HydraTime,
    pub until: HydraTime,
    pub verbosity: VerbosityLevel,
    pub max_nodes: usize,
}

/// One classified diff record for a single node.
#[derive(Debug, Clone)]
pub enum DiffRecord {
    /// Node existed at `until` but not at `since`.
    Added {
        kind: ObjectKind,
        id: HydraId,
        to_version: VersionNumber,
        end_view: Value,
    },
    /// Node existed at `since` but not at `until`.
    Removed {
        kind: ObjectKind,
        id: HydraId,
        from_version: VersionNumber,
        start_view: Value,
    },
    /// Node existed at both endpoints and the view projection changed.
    Modified {
        kind: ObjectKind,
        id: HydraId,
        from_version: VersionNumber,
        to_version: VersionNumber,
        fields: BTreeMap<String, FieldChange>,
    },
}

/// Before/after pair for a single flattened field path in a `modified` record.
#[derive(Debug, Clone)]
pub struct FieldChange {
    pub before: Value,
    pub after: Value,
}

/// Top-level entry point for `hydra graph diff`.
pub async fn run_diff(
    client: &dyn HydraClientInterface,
    params: DiffParams,
    context: &CommandContext,
) -> Result<()> {
    if let Err(msg) = check_window(params.since, params.until) {
        eprintln!("error: {msg}");
        process::exit(2);
    }

    let query = match parse_query(&params.query) {
        Ok(q) => q,
        Err(err) => {
            eprintln!("{err}");
            process::exit(2);
        }
    };

    let Resolved {
        node_ids,
        kind_filters,
    } = resolve(client, query.lower()).await?;
    if node_ids.len() > params.max_nodes {
        eprintln!(
            "error: matched node set ({}) exceeds --max-nodes ({}); narrow your selection (use --max-nodes to raise)",
            node_ids.len(),
            params.max_nodes,
        );
        process::exit(2);
    }

    let filter = build_kind_filter(&kind_filters);
    let records = diff_all(
        client,
        node_ids,
        params.since.into_inner(),
        params.until.into_inner(),
        params.verbosity,
        &filter,
    )
    .await?;

    let mut buffer = Vec::new();
    render(context.output_format, &records, &mut buffer)?;
    write_stdout(&buffer)?;
    Ok(())
}

/// Reject inverted windows. Parsing was already done by clap via [`HydraTime`].
pub(crate) fn check_window(since: HydraTime, until: HydraTime) -> Result<(), String> {
    if since.into_inner() > until.into_inner() {
        return Err(format!("--since ({since}) must be <= --until ({until})"));
    }
    Ok(())
}

/// Intersect the kind post-filter lists from the resolver into a single set.
///
/// Each list in `kind_filters` comes from one `| kind=...` stage in the
/// original query; their intersection mirrors the parser's
/// "consecutive `Kind` stages collapse to one" lowering rule for the
/// non-consecutive case. Returns `None` when no kind filter was specified
/// (all hydrated nodes pass through unchanged).
fn build_kind_filter(
    kind_filters: &[Vec<ObjectKind>],
) -> Option<std::collections::HashSet<ObjectKind>> {
    if kind_filters.is_empty() {
        return None;
    }
    let mut iter = kind_filters.iter();
    let first: std::collections::HashSet<ObjectKind> =
        iter.next().expect("non-empty").iter().copied().collect();
    let intersected = iter.fold(first, |acc, ks| {
        let next: std::collections::HashSet<ObjectKind> = ks.iter().copied().collect();
        acc.intersection(&next).copied().collect()
    });
    Some(intersected)
}

/// Concurrently fetch version histories for each id and produce diff records.
async fn diff_all(
    client: &dyn HydraClientInterface,
    ids: Vec<HydraId>,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    verbosity: VerbosityLevel,
    kind_filter: &Option<std::collections::HashSet<ObjectKind>>,
) -> Result<Vec<DiffRecord>> {
    // Drop ids whose kind is excluded by --kind before issuing any HTTP calls.
    let ids: Vec<HydraId> = ids
        .into_iter()
        .filter(|id| match (kind_filter, ObjectKind::from_id(id)) {
            (Some(allow), Some(k)) => allow.contains(&k),
            (None, _) => true,
            (_, None) => false,
        })
        .collect();

    let mut iter = ids.into_iter();
    let mut in_flight: FuturesUnordered<BoxFuture<'_, Result<Option<DiffRecord>>>> =
        FuturesUnordered::new();
    let mut records: Vec<DiffRecord> = Vec::new();

    for _ in 0..DEFAULT_HYDRATION_CONCURRENCY {
        if let Some(id) = iter.next() {
            in_flight.push(diff_one(client, id, since, until, verbosity).boxed());
        } else {
            break;
        }
    }
    while let Some(result) = in_flight.next().await {
        if let Some(record) = result.context("failed to compute diff for node")? {
            records.push(record);
        }
        if let Some(id) = iter.next() {
            in_flight.push(diff_one(client, id, since, until, verbosity).boxed());
        }
    }

    records.sort_by(|a, b| record_id(a).as_ref().cmp(record_id(b).as_ref()));
    Ok(records)
}

async fn diff_one(
    client: &dyn HydraClientInterface,
    id: HydraId,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    verbosity: VerbosityLevel,
) -> Result<Option<DiffRecord>> {
    let kind = ObjectKind::from_id(&id).ok_or_else(|| {
        anyhow!("id '{id}' does not belong to a graph object kind (expected i-/p-/d-/c- prefix)")
    })?;
    let versions = fetch_versions(client, kind, &id).await?;
    Ok(classify(&id, kind, &versions, since, until, verbosity))
}

/// Apply the diff classification rules to a single node.
pub(crate) fn classify(
    id: &HydraId,
    kind: ObjectKind,
    versions: &VersionedNode,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    verbosity: VerbosityLevel,
) -> Option<DiffRecord> {
    let v_start = versions.version_at(since);
    let v_end = versions.version_at(until);
    match (v_start, v_end) {
        (None, None) => None,
        (None, Some(end)) => Some(DiffRecord::Added {
            kind,
            id: id.clone(),
            to_version: end.version(),
            end_view: end.render(verbosity),
        }),
        (Some(start), None) => Some(DiffRecord::Removed {
            kind,
            id: id.clone(),
            from_version: start.version(),
            start_view: start.render(verbosity),
        }),
        (Some(start), Some(end)) => modified_record(id, kind, start, end, verbosity),
    }
}

fn modified_record(
    id: &HydraId,
    kind: ObjectKind,
    start: VersionView<'_>,
    end: VersionView<'_>,
    verbosity: VerbosityLevel,
) -> Option<DiffRecord> {
    let before = start.render(verbosity);
    let after = end.render(verbosity);
    if before == after {
        return None;
    }
    let mut changes: BTreeMap<String, FieldChange> = BTreeMap::new();
    diff_json("", &before, &after, &mut changes);
    if changes.is_empty() {
        return None;
    }
    Some(DiffRecord::Modified {
        kind,
        id: id.clone(),
        from_version: start.version(),
        to_version: end.version(),
        fields: changes,
    })
}

/// Recursively walk two JSON values and emit a `before`/`after` entry for
/// every leaf path that differs.
///
/// Paths use dot-separated component names; array indices flatten to their
/// numeric index (e.g. `dependencies.0.type`). When one side is missing
/// (`Value::Null`) but the other is a structure, recursion still descends
/// into the structure so each new leaf gets its own entry rather than the
/// whole subtree being recorded at the parent path.
pub(crate) fn diff_json(
    path: &str,
    before: &Value,
    after: &Value,
    out: &mut BTreeMap<String, FieldChange>,
) {
    if before == after {
        return;
    }

    let null_map = serde_json::Map::new();
    let null_arr: Vec<Value> = Vec::new();

    if before.is_object() || after.is_object() {
        let a = before.as_object().unwrap_or(&null_map);
        let b = after.as_object().unwrap_or(&null_map);
        let mut keys: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for k in a.keys() {
            keys.insert(k.as_str());
        }
        for k in b.keys() {
            keys.insert(k.as_str());
        }
        for k in keys {
            let next_path = join_path(path, k);
            let a_val = a.get(k).unwrap_or(&Value::Null);
            let b_val = b.get(k).unwrap_or(&Value::Null);
            diff_json(&next_path, a_val, b_val, out);
        }
        return;
    }
    if before.is_array() || after.is_array() {
        let a = before.as_array().unwrap_or(&null_arr);
        let b = after.as_array().unwrap_or(&null_arr);
        let len = a.len().max(b.len());
        for i in 0..len {
            let next_path = join_path(path, &i.to_string());
            let a_val = a.get(i).unwrap_or(&Value::Null);
            let b_val = b.get(i).unwrap_or(&Value::Null);
            diff_json(&next_path, a_val, b_val, out);
        }
        return;
    }
    out.insert(
        path.to_string(),
        FieldChange {
            before: before.clone(),
            after: after.clone(),
        },
    );
}

fn join_path(prefix: &str, suffix: &str) -> String {
    if prefix.is_empty() {
        suffix.to_string()
    } else {
        format!("{prefix}.{suffix}")
    }
}

fn record_id(record: &DiffRecord) -> &HydraId {
    match record {
        DiffRecord::Added { id, .. }
        | DiffRecord::Removed { id, .. }
        | DiffRecord::Modified { id, .. } => id,
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
        let value = record_to_json(record);
        serde_json::to_writer(&mut *writer, &value)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn record_to_json(record: &DiffRecord) -> Value {
    match record {
        DiffRecord::Added {
            kind,
            id,
            to_version,
            end_view,
        } => serde_json::json!({
            "change": "added",
            "kind": kind.as_str(),
            "id": id.as_ref(),
            "version": { "from": Value::Null, "to": to_version },
            "object": end_view,
        }),
        DiffRecord::Removed {
            kind,
            id,
            from_version,
            start_view,
        } => serde_json::json!({
            "change": "removed",
            "kind": kind.as_str(),
            "id": id.as_ref(),
            "version": { "from": from_version, "to": Value::Null },
            "object": start_view,
        }),
        DiffRecord::Modified {
            kind,
            id,
            from_version,
            to_version,
            fields,
        } => {
            let fields_value = Value::Object(
                fields
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            serde_json::json!({
                                "before": v.before,
                                "after": v.after,
                            }),
                        )
                    })
                    .collect(),
            );
            serde_json::json!({
                "change": "modified",
                "kind": kind.as_str(),
                "id": id.as_ref(),
                "version": { "from": from_version, "to": to_version },
                "fields": fields_value,
            })
        }
    }
}

fn render_pretty(records: &[DiffRecord], writer: &mut impl Write) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }
    for record in records {
        match record {
            DiffRecord::Added {
                kind,
                id,
                to_version,
                end_view,
            } => {
                writeln!(
                    writer,
                    "{} {} (v{}): + NEW",
                    kind.as_str(),
                    id.as_ref(),
                    to_version,
                )?;
                write_view_fields(writer, end_view)?;
            }
            DiffRecord::Removed {
                kind,
                id,
                from_version,
                start_view,
            } => {
                writeln!(
                    writer,
                    "{} {} (v{}): - REMOVED",
                    kind.as_str(),
                    id.as_ref(),
                    from_version,
                )?;
                write_view_fields(writer, start_view)?;
            }
            DiffRecord::Modified {
                kind,
                id,
                from_version,
                to_version,
                fields,
            } => {
                writeln!(
                    writer,
                    "{} {} (v{} -> v{}):",
                    kind.as_str(),
                    id.as_ref(),
                    from_version,
                    to_version,
                )?;
                for (path, change) in fields {
                    writeln!(
                        writer,
                        "  + {path}: {before} \u{2192} {after}",
                        before = change.before,
                        after = change.after,
                    )?;
                }
            }
        }
    }
    writer.flush()?;
    Ok(())
}

/// Render the top-level fields of a `view_lN` projection one per line.
///
/// Non-object views (or empty objects) emit nothing — diff records always
/// have a header line that conveys the kind + id + version.
pub(crate) fn write_view_fields(writer: &mut impl Write, view: &Value) -> Result<()> {
    if let Some(map) = view.as_object() {
        for (key, value) in map {
            writeln!(writer, "  {key}: {value}")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use hydra_common::api::v1::conversations::{
        Conversation as ApiConversation, ConversationStatus,
    };
    use hydra_common::api::v1::issues::{Issue, IssueStatus, IssueType, SessionSettings};
    use hydra_common::issues::IssueVersionRecord;
    use hydra_common::users::Username;
    use hydra_common::versioning::Versioned;
    use hydra_common::{ConversationId, IssueId};

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap() + chrono::Duration::seconds(secs)
    }

    fn sample_issue(status: IssueStatus, title: &str) -> Issue {
        Issue::new(
            IssueType::Task,
            title.to_string(),
            String::new(),
            Username::from("creator"),
            String::new(),
            status.into(),
            None,
            None,
            Some(SessionSettings::default()),
            Vec::new(),
            Vec::new(),
            false,
            None,
            None,
            None,
        )
    }

    fn issue_version_record(
        id: &IssueId,
        version: u64,
        timestamp: DateTime<Utc>,
        issue: Issue,
    ) -> IssueVersionRecord {
        IssueVersionRecord::new(
            id.clone(),
            version,
            timestamp,
            issue,
            None,
            timestamp,
            Vec::new(),
        )
    }

    #[test]
    fn check_window_rejects_since_after_until() {
        let since: HydraTime = "2026-05-15T13:00:00Z".parse().unwrap();
        let until: HydraTime = "2026-05-15T12:00:00Z".parse().unwrap();
        let err = check_window(since, until).unwrap_err();
        assert!(err.contains("must be <="), "got: {err}");
    }

    #[test]
    fn check_window_accepts_ordered() {
        let since: HydraTime = "2026-05-15T12:00:00Z".parse().unwrap();
        let until: HydraTime = "2026-05-15T13:00:00Z".parse().unwrap();
        assert!(check_window(since, until).is_ok());
    }

    #[test]
    fn classify_modified_when_field_changes_within_window() {
        let id: IssueId = "i-aaaaaa".parse().unwrap();
        let issue_v1 = sample_issue(IssueStatus::Open, "first");
        let issue_v2 = sample_issue(IssueStatus::InProgress, "first");
        let history = VersionedNode::Issue(vec![
            issue_version_record(&id, 1, ts(10), issue_v1),
            issue_version_record(&id, 2, ts(50), issue_v2),
        ]);
        let hydra_id: HydraId = id.clone().into();
        let record = classify(
            &hydra_id,
            ObjectKind::Issue,
            &history,
            ts(20),
            ts(100),
            VerbosityLevel::L1,
        )
        .expect("expected a diff record");
        match record {
            DiffRecord::Modified {
                from_version,
                to_version,
                fields,
                ..
            } => {
                assert_eq!(from_version, 1);
                assert_eq!(to_version, 2);
                let status = fields.get("status").expect("status field change");
                assert_eq!(status.before, serde_json::json!("open"));
                assert_eq!(status.after, serde_json::json!("in-progress"));
            }
            other => panic!("expected Modified, got {other:?}"),
        }
    }

    #[test]
    fn classify_added_when_first_version_is_inside_window() {
        let id: IssueId = "i-bbbbbb".parse().unwrap();
        let history = VersionedNode::Issue(vec![issue_version_record(
            &id,
            1,
            ts(100),
            sample_issue(IssueStatus::Open, "new"),
        )]);
        let hydra_id: HydraId = id.clone().into();
        let record = classify(
            &hydra_id,
            ObjectKind::Issue,
            &history,
            ts(50),
            ts(150),
            VerbosityLevel::L1,
        )
        .expect("expected a diff record");
        match record {
            DiffRecord::Added { to_version, .. } => assert_eq!(to_version, 1),
            other => panic!("expected Added, got {other:?}"),
        }
    }

    #[test]
    fn classify_unchanged_returns_none() {
        let id: IssueId = "i-cccccc".parse().unwrap();
        let history = VersionedNode::Issue(vec![issue_version_record(
            &id,
            1,
            ts(10),
            sample_issue(IssueStatus::Open, "unchanged"),
        )]);
        let hydra_id: HydraId = id.clone().into();
        let record = classify(
            &hydra_id,
            ObjectKind::Issue,
            &history,
            ts(50),
            ts(100),
            VerbosityLevel::L1,
        );
        assert!(record.is_none(), "expected no diff for unchanged");
    }

    #[test]
    fn classify_returns_none_when_no_versions_in_or_before_window() {
        let id: IssueId = "i-dddddd".parse().unwrap();
        // version timestamp is after both since and until: no match.
        let history = VersionedNode::Issue(vec![issue_version_record(
            &id,
            1,
            ts(500),
            sample_issue(IssueStatus::Open, "future"),
        )]);
        let hydra_id: HydraId = id.clone().into();
        let record = classify(
            &hydra_id,
            ObjectKind::Issue,
            &history,
            ts(0),
            ts(100),
            VerbosityLevel::L1,
        );
        assert!(record.is_none());
    }

    #[test]
    fn classify_modified_respects_verbosity_l1_vs_l2() {
        // Description changes; at L1 (title + status only) this is unchanged.
        let id: IssueId = "i-eeeeee".parse().unwrap();
        let mut issue_v1 = sample_issue(IssueStatus::Open, "same-title");
        issue_v1.description = "before".to_string();
        let mut issue_v2 = sample_issue(IssueStatus::Open, "same-title");
        issue_v2.description = "after".to_string();
        let history = VersionedNode::Issue(vec![
            issue_version_record(&id, 1, ts(10), issue_v1),
            issue_version_record(&id, 2, ts(50), issue_v2),
        ]);
        let hydra_id: HydraId = id.clone().into();

        let none = classify(
            &hydra_id,
            ObjectKind::Issue,
            &history,
            ts(20),
            ts(100),
            VerbosityLevel::L1,
        );
        // At L1, only title + status are projected, so the description change
        // is invisible.
        assert!(none.is_none(), "L1 should hide description-only change");

        let some = classify(
            &hydra_id,
            ObjectKind::Issue,
            &history,
            ts(20),
            ts(100),
            VerbosityLevel::L3,
        )
        .expect("L3 should surface the change");
        match some {
            DiffRecord::Modified { fields, .. } => {
                assert!(fields.contains_key("description"), "fields: {fields:?}");
            }
            other => panic!("expected Modified, got {other:?}"),
        }
    }

    #[test]
    fn diff_json_emits_one_entry_per_changed_leaf() {
        let before = serde_json::json!({
            "a": 1,
            "b": { "c": "x", "d": "same" },
            "e": [10, 20],
        });
        let after = serde_json::json!({
            "a": 1,
            "b": { "c": "y", "d": "same" },
            "e": [10, 21],
        });
        let mut out = BTreeMap::new();
        diff_json("", &before, &after, &mut out);
        let mut keys: Vec<&String> = out.keys().collect();
        keys.sort();
        assert_eq!(keys, vec![&"b.c".to_string(), &"e.1".to_string()]);
        assert_eq!(out["b.c"].before, serde_json::json!("x"));
        assert_eq!(out["b.c"].after, serde_json::json!("y"));
        assert_eq!(out["e.1"].before, serde_json::json!(20));
        assert_eq!(out["e.1"].after, serde_json::json!(21));
    }

    #[test]
    fn diff_json_records_added_subtree() {
        let before = serde_json::json!({ "a": 1 });
        let after = serde_json::json!({ "a": 1, "b": { "c": 2 } });
        let mut out = BTreeMap::new();
        diff_json("", &before, &after, &mut out);
        assert!(out.contains_key("b.c"), "got: {out:?}");
        assert_eq!(out["b.c"].before, Value::Null);
        assert_eq!(out["b.c"].after, serde_json::json!(2));
    }

    #[test]
    fn render_pretty_added_and_removed_include_view_fields() {
        let id_added: HydraId = "i-aaaaaa".parse::<IssueId>().unwrap().into();
        let id_removed: HydraId = "i-bbbbbb".parse::<IssueId>().unwrap().into();
        let records = vec![
            DiffRecord::Added {
                kind: ObjectKind::Issue,
                id: id_added.clone(),
                to_version: 1,
                end_view: serde_json::json!({
                    "title": "new issue",
                    "status": "open",
                }),
            },
            DiffRecord::Removed {
                kind: ObjectKind::Issue,
                id: id_removed.clone(),
                from_version: 3,
                start_view: serde_json::json!({
                    "title": "old issue",
                    "status": "closed",
                }),
            },
        ];
        let mut buf = Vec::new();
        render_pretty(&records, &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("issue i-aaaaaa (v1): + NEW"), "got: {out}");
        assert!(out.contains("  title: \"new issue\""), "got: {out}");
        assert!(out.contains("  status: \"open\""), "got: {out}");
        assert!(out.contains("issue i-bbbbbb (v3): - REMOVED"), "got: {out}");
        assert!(out.contains("  title: \"old issue\""), "got: {out}");
        assert!(out.contains("  status: \"closed\""), "got: {out}");
    }

    #[test]
    fn render_pretty_modified_keeps_field_change_list() {
        let id: HydraId = "i-cccccc".parse::<IssueId>().unwrap().into();
        let mut fields = BTreeMap::new();
        fields.insert(
            "status".to_string(),
            FieldChange {
                before: serde_json::json!("open"),
                after: serde_json::json!("in-progress"),
            },
        );
        let records = vec![DiffRecord::Modified {
            kind: ObjectKind::Issue,
            id: id.clone(),
            from_version: 1,
            to_version: 2,
            fields,
        }];
        let mut buf = Vec::new();
        render_pretty(&records, &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("issue i-cccccc (v1 -> v2):"), "got: {out}");
        assert!(out.contains("status"), "got: {out}");
        assert!(out.contains("open"), "got: {out}");
        assert!(out.contains("in-progress"), "got: {out}");
    }

    #[test]
    fn conversation_versioned_node_classifies_as_modified() {
        let conv_id = ConversationId::new();
        let make_conv = |status: ConversationStatus| {
            ApiConversation::new(
                conv_id.clone(),
                Some("t".to_string()),
                None,
                status,
                Username::from("creator"),
                SessionSettings::default(),
                ts(0),
                ts(0),
            )
        };
        let history = VersionedNode::Conversation(vec![
            Versioned::new(make_conv(ConversationStatus::Active), 1, ts(10), ts(0)),
            Versioned::new(make_conv(ConversationStatus::Closed), 2, ts(50), ts(0)),
        ]);
        let hydra_id: HydraId = conv_id.clone().into();
        let record = classify(
            &hydra_id,
            ObjectKind::Conversation,
            &history,
            ts(20),
            ts(100),
            VerbosityLevel::L1,
        )
        .expect("expected Modified");
        match record {
            DiffRecord::Modified {
                from_version,
                to_version,
                fields,
                ..
            } => {
                assert_eq!(from_version, 1);
                assert_eq!(to_version, 2);
                assert!(fields.contains_key("status"), "fields: {fields:?}");
            }
            other => panic!("expected Modified, got {other:?}"),
        }
    }
}
