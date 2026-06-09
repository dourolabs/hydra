//! `hydra graph log` — implementation.
//!
//! Selects nodes via the shared pipe-form DSL resolver, then for each node
//! walks its version history and emits a `created` or `updated` event for
//! every version whose timestamp falls in `(--since, --until]`. Events
//! from all matched nodes are merged and sorted most-recent-first, then
//! truncated to `--limit`.

use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::process;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};
use futures::FutureExt;
use hydra_common::actor_ref::ActorRef;
use hydra_common::graph::{ObjectKind, VerbosityLevel};
use hydra_common::time::HydraTime;
use hydra_common::versioning::VersionNumber;
use hydra_common::HydraId;
use serde_json::Value;

use crate::client::HydraClientInterface;
use crate::command::graph::diff::{check_window, diff_json, write_view_fields, FieldChange};
use crate::command::graph::dispatch::{fetch_versions, VersionedNode};
use crate::command::graph::query::parse as parse_query;
use crate::command::graph::resolver::{resolve, Resolved};
use crate::command::graph::DEFAULT_HYDRATION_CONCURRENCY;
use crate::command::output::{CommandContext, ResolvedOutputFormat};
use crate::output_writer::write_stdout;

/// Selection and rendering inputs for `hydra graph log`.
#[derive(Debug, Clone)]
pub struct LogParams {
    /// Pipe-grammar query string. Parsed at the top of [`run_log`]; a parse
    /// error exits with code 2 and the caret-quoted message.
    pub query: String,
    pub since: HydraTime,
    pub until: HydraTime,
    pub verbosity: VerbosityLevel,
    pub limit: usize,
    pub max_nodes: usize,
}

/// One log event for a single node version.
#[derive(Debug, Clone)]
pub enum LogEvent {
    /// First version of the node, falling inside the time window.
    Created {
        kind: ObjectKind,
        id: HydraId,
        version: VersionNumber,
        ts: DateTime<Utc>,
        actor: Option<ActorRef>,
        object: Value,
    },
    /// A subsequent version inside the window. `changes` is the JSON-diff of
    /// the previous version's projection against the current version's,
    /// produced by [`diff_json`].
    Updated {
        kind: ObjectKind,
        id: HydraId,
        version: VersionNumber,
        ts: DateTime<Utc>,
        actor: Option<ActorRef>,
        changes: BTreeMap<String, FieldChange>,
    },
}

impl LogEvent {
    fn timestamp(&self) -> DateTime<Utc> {
        match self {
            LogEvent::Created { ts, .. } | LogEvent::Updated { ts, .. } => *ts,
        }
    }

    fn id(&self) -> &HydraId {
        match self {
            LogEvent::Created { id, .. } | LogEvent::Updated { id, .. } => id,
        }
    }

    fn version(&self) -> VersionNumber {
        match self {
            LogEvent::Created { version, .. } | LogEvent::Updated { version, .. } => *version,
        }
    }

    fn to_json(&self) -> Value {
        match self {
            LogEvent::Created {
                kind,
                id,
                version,
                ts,
                actor,
                object,
            } => serde_json::json!({
                "event": "created",
                "kind": kind.as_str(),
                "id": id.as_ref(),
                "version": version,
                "ts": ts.to_rfc3339(),
                "actor": actor_json(actor),
                "object": object,
            }),
            LogEvent::Updated {
                kind,
                id,
                version,
                ts,
                actor,
                changes,
            } => {
                let changes_value = Value::Object(
                    changes
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
                    "event": "updated",
                    "kind": kind.as_str(),
                    "id": id.as_ref(),
                    "version": version,
                    "ts": ts.to_rfc3339(),
                    "actor": actor_json(actor),
                    "changes": changes_value,
                })
            }
        }
    }
}

/// Top-level entry point for `hydra graph log`.
pub async fn run_log(
    client: &dyn HydraClientInterface,
    params: LogParams,
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
    let mut events = collect_events(
        client,
        node_ids,
        params.since.into_inner(),
        params.until.into_inner(),
        params.verbosity,
        &filter,
    )
    .await?;

    sort_and_truncate(&mut events, params.limit);

    let mut buffer = Vec::new();
    render(context.output_format, &events, &mut buffer)?;
    write_stdout(&buffer)?;
    Ok(())
}

/// Intersect the kind post-filter lists from the resolver into a single set.
///
/// Mirrors `diff::build_kind_filter`: each list in `kind_filters` comes from
/// one `| kind=...` stage in the original query; their intersection mirrors
/// the parser's "consecutive `Kind` stages collapse to one" lowering rule for
/// the non-consecutive case. Returns `None` when no kind filter was specified
/// (all hydrated nodes pass through unchanged).
fn build_kind_filter(kind_filters: &[Vec<ObjectKind>]) -> Option<HashSet<ObjectKind>> {
    if kind_filters.is_empty() {
        return None;
    }
    let mut iter = kind_filters.iter();
    let first: HashSet<ObjectKind> = iter.next().expect("non-empty").iter().copied().collect();
    let intersected = iter.fold(first, |acc, ks| {
        let next: HashSet<ObjectKind> = ks.iter().copied().collect();
        acc.intersection(&next).copied().collect()
    });
    Some(intersected)
}

/// Sort events most-recent-first; ties broken by id ascending for
/// determinism. Then truncate to `limit`.
pub(crate) fn sort_and_truncate(events: &mut Vec<LogEvent>, limit: usize) {
    events.sort_by(|a, b| {
        b.timestamp()
            .cmp(&a.timestamp())
            .then_with(|| a.id().as_ref().cmp(b.id().as_ref()))
            .then_with(|| a.version().cmp(&b.version()))
    });
    events.truncate(limit);
}

/// Concurrently fetch version histories for each id and turn them into
/// in-window events.
async fn collect_events(
    client: &dyn HydraClientInterface,
    ids: Vec<HydraId>,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    verbosity: VerbosityLevel,
    kind_filter: &Option<HashSet<ObjectKind>>,
) -> Result<Vec<LogEvent>> {
    // Drop ids whose kind is excluded by the `kind=...` post-filter (or whose
    // prefix doesn't map to a known kind at all) before issuing any HTTP calls.
    let ids: Vec<HydraId> = ids
        .into_iter()
        .filter(|id| match (kind_filter, ObjectKind::from_id(id)) {
            (Some(allow), Some(k)) => allow.contains(&k),
            (None, Some(_)) => true,
            (_, None) => false,
        })
        .collect();

    let mut iter = ids.into_iter();
    let mut in_flight: FuturesUnordered<BoxFuture<'_, Result<Vec<LogEvent>>>> =
        FuturesUnordered::new();
    let mut out: Vec<LogEvent> = Vec::new();

    for _ in 0..DEFAULT_HYDRATION_CONCURRENCY {
        if let Some(id) = iter.next() {
            in_flight.push(events_for_one(client, id, since, until, verbosity).boxed());
        } else {
            break;
        }
    }
    while let Some(result) = in_flight.next().await {
        let events = result.context("failed to compute log events for node")?;
        out.extend(events);
        if let Some(id) = iter.next() {
            in_flight.push(events_for_one(client, id, since, until, verbosity).boxed());
        }
    }
    Ok(out)
}

async fn events_for_one(
    client: &dyn HydraClientInterface,
    id: HydraId,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    verbosity: VerbosityLevel,
) -> Result<Vec<LogEvent>> {
    let kind = ObjectKind::from_id(&id).ok_or_else(|| {
        anyhow!("id '{id}' does not belong to a graph object kind (expected i-/p-/d-/c- prefix)")
    })?;
    let versions = fetch_versions(client, kind, &id).await?;
    Ok(events_in_window(
        &id, kind, &versions, since, until, verbosity,
    ))
}

/// Walk the version vector and emit one [`LogEvent`] per version whose
/// timestamp lies in `(since, until]`. The first version (overall) gets a
/// `Created` event; everything else gets `Updated` with a JSON-diff against
/// the immediately preceding version.
pub(crate) fn events_in_window(
    id: &HydraId,
    kind: ObjectKind,
    versions: &VersionedNode,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    verbosity: VerbosityLevel,
) -> Vec<LogEvent> {
    let mut out: Vec<LogEvent> = Vec::new();
    let mut prev_view: Option<Value> = None;
    for cur in versions.iter_views() {
        let ts = cur.timestamp();
        let cur_view = cur.render(verbosity);
        if ts > since && ts <= until {
            match prev_view.as_ref() {
                None => {
                    out.push(LogEvent::Created {
                        kind,
                        id: id.clone(),
                        version: cur.version(),
                        ts,
                        actor: cur.actor().cloned(),
                        object: cur_view.clone(),
                    });
                }
                Some(prev_json) => {
                    let mut changes: BTreeMap<String, FieldChange> = BTreeMap::new();
                    diff_json("", prev_json, &cur_view, &mut changes);
                    out.push(LogEvent::Updated {
                        kind,
                        id: id.clone(),
                        version: cur.version(),
                        ts,
                        actor: cur.actor().cloned(),
                        changes,
                    });
                }
            }
        }
        prev_view = Some(cur_view);
    }
    out
}

fn render(
    format: ResolvedOutputFormat,
    events: &[LogEvent],
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => render_jsonl(events, writer),
        ResolvedOutputFormat::Pretty => render_pretty(events, writer),
    }
}

fn render_jsonl(events: &[LogEvent], writer: &mut impl Write) -> Result<()> {
    for event in events {
        let value = event.to_json();
        serde_json::to_writer(&mut *writer, &value)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

/// JSON serialization for the `actor` field on JSONL log records.
///
/// Returns `Value::Null` when no actor is recorded, otherwise the short-form
/// string produced by [`ActorRef::display_name`]. The mapping per variant is:
/// - `Authenticated(Username(u))` → `"<username>"`
/// - `Authenticated(Session(s))`  → `"s-…"`
/// - `Authenticated(Issue(i))`    → `"i-…"`
/// - `Authenticated(Service(n))`  → `"svc-<name>"`
/// - `System { worker_name, on_behalf_of }`      → `"<worker_name>"` (with " (on behalf of …)" if present)
/// - `Automation { automation_name, triggered_by }` → `"<automation_name>"` (with " (triggered by …)" if present)
fn actor_json(actor: &Option<ActorRef>) -> Value {
    match actor {
        Some(a) => Value::String(a.display_name()),
        None => Value::Null,
    }
}

fn render_pretty(events: &[LogEvent], writer: &mut impl Write) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    for event in events {
        match event {
            LogEvent::Created {
                kind,
                id,
                version,
                ts,
                actor,
                object,
            } => {
                writeln!(
                    writer,
                    "{ts} {kind} {id} v{version} CREATED by {actor}",
                    ts = ts.to_rfc3339(),
                    kind = kind.as_str(),
                    id = id.as_ref(),
                    actor = actor_display(actor),
                )?;
                write_view_fields(writer, object)?;
            }
            LogEvent::Updated {
                kind,
                id,
                version,
                ts,
                actor,
                changes,
            } => {
                writeln!(
                    writer,
                    "{ts} {kind} {id} v{version} UPDATED by {actor}",
                    ts = ts.to_rfc3339(),
                    kind = kind.as_str(),
                    id = id.as_ref(),
                    actor = actor_display(actor),
                )?;
                for (path, change) in changes {
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

fn actor_display(actor: &Option<ActorRef>) -> String {
    match actor {
        Some(a) => a.display_name(),
        None => "<unknown>".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use hydra_common::actor_ref::{ActorId, ActorRef};
    use hydra_common::api::v1::conversations::{
        Conversation as ApiConversation, ConversationStatus,
    };
    use hydra_common::api::v1::issues::{Issue, IssueStatus, IssueType, SessionSettings};
    use hydra_common::issues::IssueVersionRecord;
    use hydra_common::test_utils::status::make_status_def;
    use hydra_common::users::Username;
    use hydra_common::versioning::Versioned;
    use hydra_common::{ConversationId, IssueId, ProjectId, SessionId};
    use std::str::FromStr;

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
            make_status_def(status.into()),
            ProjectId::default_project(),
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

    fn session_actor(name: &str) -> ActorRef {
        let session_id = SessionId::from_str(name).expect("valid session id");
        ActorRef::Authenticated {
            actor_id: ActorId::Adhoc(session_id),
            session_id: None,
        }
    }

    fn issue_version_record(
        id: &IssueId,
        version: u64,
        timestamp: DateTime<Utc>,
        issue: Issue,
        actor: Option<ActorRef>,
    ) -> IssueVersionRecord {
        IssueVersionRecord::new(
            id.clone(),
            version,
            timestamp,
            issue,
            actor,
            timestamp,
            Vec::new(),
        )
    }

    #[test]
    fn events_in_window_emits_created_then_updated_for_in_window_versions() {
        let id: IssueId = "i-aaaaaa".parse().unwrap();
        let actor = session_actor("s-abcdef");
        let history = VersionedNode::Issue(vec![
            // v1 outside window, v2 inside, v3 inside, v4 inside, v5 outside
            issue_version_record(
                &id,
                1,
                ts(-100),
                sample_issue(IssueStatus::Open, "t1"),
                Some(actor.clone()),
            ),
            issue_version_record(
                &id,
                2,
                ts(10),
                sample_issue(IssueStatus::InProgress, "t2"),
                Some(actor.clone()),
            ),
            issue_version_record(
                &id,
                3,
                ts(20),
                sample_issue(IssueStatus::InProgress, "t3"),
                Some(actor.clone()),
            ),
            issue_version_record(
                &id,
                4,
                ts(30),
                sample_issue(IssueStatus::Closed, "t3"),
                Some(actor.clone()),
            ),
            issue_version_record(
                &id,
                5,
                ts(1000),
                sample_issue(IssueStatus::Closed, "t3"),
                Some(actor.clone()),
            ),
        ]);
        let hydra_id: HydraId = id.clone().into();
        let events = events_in_window(
            &hydra_id,
            ObjectKind::Issue,
            &history,
            ts(0),
            ts(100),
            VerbosityLevel::L3,
        );
        // v2, v3, v4 are inside the window (3 events).
        assert_eq!(events.len(), 3, "got {events:?}");
        // All should be Updated (since prev exists for each).
        for event in &events {
            assert!(
                matches!(event, LogEvent::Updated { .. }),
                "expected Updated: {event:?}"
            );
        }
    }

    #[test]
    fn events_in_window_emits_created_when_earliest_in_window_is_first_version() {
        let id: IssueId = "i-bbbbbb".parse().unwrap();
        let history = VersionedNode::Issue(vec![
            issue_version_record(&id, 1, ts(10), sample_issue(IssueStatus::Open, "t1"), None),
            issue_version_record(
                &id,
                2,
                ts(20),
                sample_issue(IssueStatus::InProgress, "t1"),
                None,
            ),
        ]);
        let hydra_id: HydraId = id.clone().into();
        let events = events_in_window(
            &hydra_id,
            ObjectKind::Issue,
            &history,
            ts(0),
            ts(100),
            VerbosityLevel::L1,
        );
        assert_eq!(events.len(), 2);
        match &events[0] {
            LogEvent::Created { version, .. } => assert_eq!(*version, 1),
            other => panic!("expected Created, got {other:?}"),
        }
        match &events[1] {
            LogEvent::Updated {
                version, changes, ..
            } => {
                assert_eq!(*version, 2);
                assert!(changes.contains_key("status"), "got changes: {changes:?}");
            }
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn events_in_window_skips_versions_outside_range() {
        let id: IssueId = "i-cccccc".parse().unwrap();
        let history = VersionedNode::Issue(vec![
            issue_version_record(
                &id,
                1,
                ts(-100),
                sample_issue(IssueStatus::Open, "t1"),
                None,
            ),
            issue_version_record(
                &id,
                2,
                ts(1000),
                sample_issue(IssueStatus::Closed, "t1"),
                None,
            ),
        ]);
        let hydra_id: HydraId = id.clone().into();
        let events = events_in_window(
            &hydra_id,
            ObjectKind::Issue,
            &history,
            ts(0),
            ts(100),
            VerbosityLevel::L1,
        );
        assert!(events.is_empty(), "got {events:?}");
    }

    #[test]
    fn events_in_window_excludes_boundary_since_includes_until() {
        let id: IssueId = "i-dddddd".parse().unwrap();
        let history = VersionedNode::Issue(vec![
            issue_version_record(&id, 1, ts(0), sample_issue(IssueStatus::Open, "t"), None),
            issue_version_record(
                &id,
                2,
                ts(100),
                sample_issue(IssueStatus::InProgress, "t"),
                None,
            ),
        ]);
        let hydra_id: HydraId = id.clone().into();
        let events = events_in_window(
            &hydra_id,
            ObjectKind::Issue,
            &history,
            ts(0),
            ts(100),
            VerbosityLevel::L1,
        );
        assert_eq!(events.len(), 1);
        // v1 at ts(0) is excluded (open interval on `since`); v2 at ts(100) is
        // included (closed interval on `until`).
        match &events[0] {
            LogEvent::Updated { version, .. } => assert_eq!(*version, 2),
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn events_in_window_honors_verbosity() {
        // Description-only change is invisible at L1 but visible at L3.
        let id: IssueId = "i-eeeeee".parse().unwrap();
        let mut v1 = sample_issue(IssueStatus::Open, "same-title");
        v1.description = "before".to_string();
        let mut v2 = sample_issue(IssueStatus::Open, "same-title");
        v2.description = "after".to_string();
        let history = VersionedNode::Issue(vec![
            issue_version_record(&id, 1, ts(-100), v1, None),
            issue_version_record(&id, 2, ts(10), v2, None),
        ]);
        let hydra_id: HydraId = id.clone().into();

        let events_l1 = events_in_window(
            &hydra_id,
            ObjectKind::Issue,
            &history,
            ts(0),
            ts(100),
            VerbosityLevel::L1,
        );
        assert_eq!(events_l1.len(), 1);
        match &events_l1[0] {
            LogEvent::Updated { changes, .. } => {
                assert!(
                    changes.is_empty(),
                    "L1 should hide description-only change: {changes:?}",
                );
            }
            other => panic!("expected Updated, got {other:?}"),
        }

        let events_l3 = events_in_window(
            &hydra_id,
            ObjectKind::Issue,
            &history,
            ts(0),
            ts(100),
            VerbosityLevel::L3,
        );
        assert_eq!(events_l3.len(), 1);
        match &events_l3[0] {
            LogEvent::Updated { changes, .. } => {
                assert!(
                    changes.contains_key("description"),
                    "L3 should surface description change: {changes:?}",
                );
            }
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn sort_and_truncate_orders_most_recent_first_and_caps_limit() {
        let id_a: HydraId = "i-aaaaaa".parse::<IssueId>().unwrap().into();
        let id_b: HydraId = "i-bbbbbb".parse::<IssueId>().unwrap().into();
        let make_event = |id: &HydraId, ts: DateTime<Utc>, v: VersionNumber| LogEvent::Updated {
            kind: ObjectKind::Issue,
            id: id.clone(),
            version: v,
            ts,
            actor: None,
            changes: BTreeMap::new(),
        };
        let mut events = vec![
            make_event(&id_a, ts(10), 1),
            make_event(&id_b, ts(30), 1),
            make_event(&id_a, ts(20), 2),
            make_event(&id_b, ts(40), 2),
        ];
        sort_and_truncate(&mut events, 100);
        let tss: Vec<_> = events.iter().map(|e| e.timestamp()).collect();
        assert_eq!(tss, vec![ts(40), ts(30), ts(20), ts(10)]);
        sort_and_truncate(&mut events, 2);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].timestamp(), ts(40));
        assert_eq!(events[1].timestamp(), ts(30));
    }

    #[test]
    fn conversation_log_walks_event_fold_history() {
        let conv_id = ConversationId::new();
        let make_conv = |status: ConversationStatus| {
            ApiConversation::new(
                conv_id.clone(),
                Some("t".to_string()),
                None,
                status,
                Username::from("creator"),
                SessionSettings::default(),
                None,
                ts(0),
                ts(0),
            )
        };
        let history = VersionedNode::Conversation(vec![
            Versioned::new(make_conv(ConversationStatus::Active), 1, ts(10), ts(0)),
            Versioned::new(make_conv(ConversationStatus::Closed), 2, ts(50), ts(0)),
        ]);
        let hydra_id: HydraId = conv_id.clone().into();
        let events = events_in_window(
            &hydra_id,
            ObjectKind::Conversation,
            &history,
            ts(0),
            ts(100),
            VerbosityLevel::L1,
        );
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], LogEvent::Created { .. }));
        match &events[1] {
            LogEvent::Updated { changes, .. } => {
                assert!(changes.contains_key("status"), "got: {changes:?}");
            }
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn event_to_json_created_includes_object_and_actor() {
        let id_a: HydraId = "i-aaaaaa".parse::<IssueId>().unwrap().into();
        let event = LogEvent::Created {
            kind: ObjectKind::Issue,
            id: id_a.clone(),
            version: 1,
            ts: ts(10),
            actor: Some(session_actor("s-abcdef")),
            object: serde_json::json!({ "title": "t", "status": "open" }),
        };
        let value = event.to_json();
        assert_eq!(value["event"], "created");
        assert_eq!(value["kind"], "issue");
        assert_eq!(value["id"], id_a.as_ref());
        assert_eq!(value["version"], 1);
        assert_eq!(value["actor"], "s-abcdef");
        assert_eq!(value["object"]["title"], "t");
    }

    #[test]
    fn render_pretty_created_includes_view_fields() {
        let id_a: HydraId = "i-aaaaaa".parse::<IssueId>().unwrap().into();
        let events = vec![LogEvent::Created {
            kind: ObjectKind::Issue,
            id: id_a.clone(),
            version: 1,
            ts: ts(10),
            actor: Some(session_actor("s-abcdef")),
            object: serde_json::json!({
                "title": "first issue",
                "status": "open",
            }),
        }];
        let mut buf = Vec::new();
        render_pretty(&events, &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("issue i-aaaaaa v1 CREATED"), "got: {out}");
        assert!(out.contains("  title: \"first issue\""), "got: {out}");
        assert!(out.contains("  status: \"open\""), "got: {out}");
    }

    #[test]
    fn render_pretty_updated_keeps_change_list() {
        let id_a: HydraId = "i-aaaaaa".parse::<IssueId>().unwrap().into();
        let mut changes = BTreeMap::new();
        changes.insert(
            "status".to_string(),
            FieldChange {
                before: serde_json::json!("open"),
                after: serde_json::json!("in-progress"),
            },
        );
        let events = vec![LogEvent::Updated {
            kind: ObjectKind::Issue,
            id: id_a.clone(),
            version: 2,
            ts: ts(20),
            actor: None,
            changes,
        }];
        let mut buf = Vec::new();
        render_pretty(&events, &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("issue i-aaaaaa v2 UPDATED"), "got: {out}");
        assert!(out.contains("status"), "got: {out}");
        assert!(out.contains("open"), "got: {out}");
        assert!(out.contains("in-progress"), "got: {out}");
    }

    #[test]
    fn event_to_json_updated_has_changes_payload() {
        let id_a: HydraId = "i-aaaaaa".parse::<IssueId>().unwrap().into();
        let mut changes = BTreeMap::new();
        changes.insert(
            "status".to_string(),
            FieldChange {
                before: serde_json::json!("open"),
                after: serde_json::json!("in-progress"),
            },
        );
        let event = LogEvent::Updated {
            kind: ObjectKind::Issue,
            id: id_a.clone(),
            version: 2,
            ts: ts(20),
            actor: None,
            changes,
        };
        let value = event.to_json();
        assert_eq!(value["event"], "updated");
        assert_eq!(value["changes"]["status"]["before"], "open");
        assert_eq!(value["changes"]["status"]["after"], "in-progress");
        assert!(value["actor"].is_null());
    }

    #[test]
    fn event_to_json_actor_for_system_variant_uses_worker_name() {
        let id_a: HydraId = "i-aaaaaa".parse::<IssueId>().unwrap().into();
        let actor = ActorRef::System {
            worker_name: "worker".to_string(),
            on_behalf_of: None,
        };
        let event = LogEvent::Created {
            kind: ObjectKind::Issue,
            id: id_a,
            version: 1,
            ts: ts(10),
            actor: Some(actor),
            object: serde_json::json!({}),
        };
        let value = event.to_json();
        assert_eq!(value["actor"], "worker");
    }
}
