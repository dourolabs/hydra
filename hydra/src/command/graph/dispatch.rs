//! Hydrated-node dispatch: id-prefix → `GET /v1/{kind}/:id` → `HydratedNode`,
//! plus the `render_view` shim that fans out to per-kind `GraphView::view_lN`.
//!
//! Also exposes `VersionedNode` + `fetch_versions` for `hydra graph diff`
//! (PR 4) and `hydra graph log` (PR 5).

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use hydra_common::api::v1::conversations::Conversation as ApiConversation;
use hydra_common::conversation::events_to_versions;
use hydra_common::documents::DocumentVersionRecord;
use hydra_common::graph::{GraphView, ObjectKind, VerbosityLevel};
use hydra_common::issues::IssueVersionRecord;
use hydra_common::patches::PatchVersionRecord;
use hydra_common::versioning::{VersionNumber, Versioned};
use hydra_common::HydraId;
use serde_json::Value;

use crate::client::HydraClientInterface;

/// A hydrated object retrieved by id-prefix dispatch.
///
/// One variant per `ObjectKind`. PRs 4 and 5 may add `VersionedObject<T>`
/// variants alongside these; we keep the naming simple here (one variant per
/// kind) so the dispatch path stays a one-line match.
#[derive(Debug, Clone)]
pub enum HydratedNode {
    Issue(IssueVersionRecord),
    Patch(PatchVersionRecord),
    Document(DocumentVersionRecord),
    Conversation(ApiConversation),
}

impl HydratedNode {
    pub fn id(&self) -> HydraId {
        match self {
            HydratedNode::Issue(r) => r.issue_id.clone().into(),
            HydratedNode::Patch(r) => r.patch_id.clone().into(),
            HydratedNode::Document(r) => r.document_id.clone().into(),
            HydratedNode::Conversation(c) => c.conversation_id.clone().into(),
        }
    }

    pub fn kind(&self) -> ObjectKind {
        match self {
            HydratedNode::Issue(_) => ObjectKind::Issue,
            HydratedNode::Patch(_) => ObjectKind::Patch,
            HydratedNode::Document(_) => ObjectKind::Document,
            HydratedNode::Conversation(_) => ObjectKind::Conversation,
        }
    }

    pub fn kind_str(&self) -> &'static str {
        kind_to_str(self.kind())
    }
}

pub fn kind_to_str(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Issue => "issue",
        ObjectKind::Patch => "patch",
        ObjectKind::Document => "document",
        ObjectKind::Conversation => "conversation",
    }
}

/// Render a hydrated node through the per-kind `GraphView::view_lN` projection.
///
/// This is the single place where `match HydratedNode` × `match VerbosityLevel`
/// lives. New kinds = one new variant on `HydratedNode` + one new arm here.
pub fn render_view(node: &HydratedNode, level: VerbosityLevel) -> Value {
    match node {
        HydratedNode::Issue(r) => match level {
            VerbosityLevel::L1 => r.issue.view_l1(),
            VerbosityLevel::L2 => r.issue.view_l2(),
            VerbosityLevel::L3 => r.issue.view_l3(),
        },
        HydratedNode::Patch(r) => match level {
            VerbosityLevel::L1 => r.patch.view_l1(),
            VerbosityLevel::L2 => r.patch.view_l2(),
            VerbosityLevel::L3 => r.patch.view_l3(),
        },
        HydratedNode::Document(r) => match level {
            VerbosityLevel::L1 => r.document.view_l1(),
            VerbosityLevel::L2 => r.document.view_l2(),
            VerbosityLevel::L3 => r.document.view_l3(),
        },
        HydratedNode::Conversation(c) => match level {
            VerbosityLevel::L1 => c.view_l1(),
            VerbosityLevel::L2 => c.view_l2(),
            VerbosityLevel::L3 => c.view_l3(),
        },
    }
}

/// Hydrate a single id by dispatching on its prefix to the matching
/// `GET /v1/{kind}/:id` endpoint.
pub async fn hydrate_by_id(
    client: &dyn HydraClientInterface,
    id: &HydraId,
) -> Result<HydratedNode> {
    if let Some(issue_id) = id.as_issue_id() {
        let record = client.get_issue(&issue_id, false).await?;
        return Ok(HydratedNode::Issue(record));
    }
    if let Some(patch_id) = id.as_patch_id() {
        let record = client.get_patch(&patch_id).await?;
        return Ok(HydratedNode::Patch(record));
    }
    if let Some(doc_id) = id.as_document_id() {
        let record = client.get_document(&doc_id, false).await?;
        return Ok(HydratedNode::Document(record));
    }
    if let Some(conv_id) = id.as_conversation_id() {
        let conversation = client.get_conversation(&conv_id).await?;
        return Ok(HydratedNode::Conversation(conversation));
    }
    Err(anyhow!(
        "id '{id}' does not belong to a graph object kind (expected i-/p-/d-/c- prefix)"
    ))
}

/// Version history of a hydrated node, one variant per `ObjectKind`.
///
/// The CLI fetches version history per-kind through different endpoints
/// (`/v1/issues/:id/versions`, `/v1/patches/:id/versions`,
/// `/v1/documents/:id/versions`, and — for conversations — the event stream
/// at `/v1/conversations/:id/events` folded client-side). The variants here
/// preserve the per-kind types so renderer code can call the matching
/// `GraphView` impl without an extra erasure step.
#[derive(Debug, Clone)]
pub enum VersionedNode {
    Issue(Vec<IssueVersionRecord>),
    Patch(Vec<PatchVersionRecord>),
    Document(Vec<DocumentVersionRecord>),
    Conversation(Vec<Versioned<ApiConversation>>),
}

impl VersionedNode {
    /// Number of versions in the underlying history.
    pub fn len(&self) -> usize {
        match self {
            VersionedNode::Issue(v) => v.len(),
            VersionedNode::Patch(v) => v.len(),
            VersionedNode::Document(v) => v.len(),
            VersionedNode::Conversation(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn kind(&self) -> ObjectKind {
        match self {
            VersionedNode::Issue(_) => ObjectKind::Issue,
            VersionedNode::Patch(_) => ObjectKind::Patch,
            VersionedNode::Document(_) => ObjectKind::Document,
            VersionedNode::Conversation(_) => ObjectKind::Conversation,
        }
    }
}

/// Fetch the full version history of a node from the server.
///
/// `kind` must match the prefix of `id`; this is asserted in debug builds and
/// errored on in release builds. Conversation versions are derived client-side
/// by fetching the initial snapshot via `GET /v1/conversations/:id` plus the
/// event stream via `GET /v1/conversations/:id/events`, then folded via
/// [`hydra_common::conversation::events_to_versions`].
pub async fn fetch_versions(
    client: &dyn HydraClientInterface,
    kind: ObjectKind,
    id: &HydraId,
) -> Result<VersionedNode> {
    match kind {
        ObjectKind::Issue => {
            let issue_id = id.as_issue_id().ok_or_else(|| {
                anyhow!("id '{id}' does not match expected kind 'issue' (i- prefix)")
            })?;
            let response = client.list_issue_versions(&issue_id).await?;
            Ok(VersionedNode::Issue(response.versions))
        }
        ObjectKind::Patch => {
            let patch_id = id.as_patch_id().ok_or_else(|| {
                anyhow!("id '{id}' does not match expected kind 'patch' (p- prefix)")
            })?;
            let response = client.list_patch_versions(&patch_id).await?;
            Ok(VersionedNode::Patch(response.versions))
        }
        ObjectKind::Document => {
            let doc_id = id.as_document_id().ok_or_else(|| {
                anyhow!("id '{id}' does not match expected kind 'document' (d- prefix)")
            })?;
            let response = client.list_document_versions(&doc_id).await?;
            Ok(VersionedNode::Document(response.versions))
        }
        ObjectKind::Conversation => {
            let conv_id = id.as_conversation_id().ok_or_else(|| {
                anyhow!("id '{id}' does not match expected kind 'conversation' (c- prefix)")
            })?;
            let initial = client.get_conversation(&conv_id).await?;
            let events = client.get_conversation_events(&conv_id).await?;
            // The CLI-facing events endpoint strips off versioning, so we
            // synthesize `Versioned<ConversationEvent>` wrappers using the
            // event index + inline timestamp before folding into snapshots.
            let creation_time = initial.created_at;
            let versioned_events: Vec<Versioned<_>> = events
                .into_iter()
                .enumerate()
                .map(|(i, event)| {
                    let ts = conversation_event_timestamp(&event);
                    Versioned::new(event, (i + 1) as VersionNumber, ts, creation_time)
                })
                .collect();
            let snapshots = events_to_versions(&initial, &versioned_events);
            Ok(VersionedNode::Conversation(snapshots))
        }
    }
}

fn conversation_event_timestamp(
    event: &hydra_common::api::v1::conversations::ConversationEvent,
) -> DateTime<Utc> {
    use hydra_common::api::v1::conversations::ConversationEvent as E;
    match event {
        E::UserMessage { timestamp, .. } => *timestamp,
        E::AssistantMessage { timestamp, .. } => *timestamp,
        E::Suspending { timestamp, .. } => *timestamp,
        E::Resumed { timestamp, .. } => *timestamp,
        E::Closed { timestamp } => *timestamp,
    }
}

/// Identifies a single version within a [`VersionedNode`] for diff classification.
#[derive(Debug, Clone, Copy)]
pub struct VersionSelection {
    pub version: VersionNumber,
    pub timestamp: DateTime<Utc>,
    pub index: usize,
}

/// Return the latest version whose timestamp is `<= at`, or `None` if no such
/// version exists. The versions vector is assumed to be ordered by ascending
/// `timestamp` (this is true for all four kinds: server-side stores append
/// versions in order, and the conversation fold preserves event order).
pub fn select_version_at(node: &VersionedNode, at: DateTime<Utc>) -> Option<VersionSelection> {
    match node {
        VersionedNode::Issue(v) => v
            .iter()
            .enumerate()
            .rev()
            .find(|(_, r)| r.timestamp <= at)
            .map(|(i, r)| VersionSelection {
                version: r.version,
                timestamp: r.timestamp,
                index: i,
            }),
        VersionedNode::Patch(v) => v
            .iter()
            .enumerate()
            .rev()
            .find(|(_, r)| r.timestamp <= at)
            .map(|(i, r)| VersionSelection {
                version: r.version,
                timestamp: r.timestamp,
                index: i,
            }),
        VersionedNode::Document(v) => v
            .iter()
            .enumerate()
            .rev()
            .find(|(_, r)| r.timestamp <= at)
            .map(|(i, r)| VersionSelection {
                version: r.version,
                timestamp: r.timestamp,
                index: i,
            }),
        VersionedNode::Conversation(v) => v
            .iter()
            .enumerate()
            .rev()
            .find(|(_, r)| r.timestamp <= at)
            .map(|(i, r)| VersionSelection {
                version: r.version,
                timestamp: r.timestamp,
                index: i,
            }),
    }
}

/// Project the version at `index` of `node` through the per-kind
/// `GraphView::view_lN` projection.
pub fn render_version(node: &VersionedNode, index: usize, level: VerbosityLevel) -> Value {
    match node {
        VersionedNode::Issue(v) => match level {
            VerbosityLevel::L1 => v[index].issue.view_l1(),
            VerbosityLevel::L2 => v[index].issue.view_l2(),
            VerbosityLevel::L3 => v[index].issue.view_l3(),
        },
        VersionedNode::Patch(v) => match level {
            VerbosityLevel::L1 => v[index].patch.view_l1(),
            VerbosityLevel::L2 => v[index].patch.view_l2(),
            VerbosityLevel::L3 => v[index].patch.view_l3(),
        },
        VersionedNode::Document(v) => match level {
            VerbosityLevel::L1 => v[index].document.view_l1(),
            VerbosityLevel::L2 => v[index].document.view_l2(),
            VerbosityLevel::L3 => v[index].document.view_l3(),
        },
        VersionedNode::Conversation(v) => match level {
            VerbosityLevel::L1 => v[index].item.view_l1(),
            VerbosityLevel::L2 => v[index].item.view_l2(),
            VerbosityLevel::L3 => v[index].item.view_l3(),
        },
    }
}
