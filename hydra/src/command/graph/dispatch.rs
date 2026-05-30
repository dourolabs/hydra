//! Hydrated-node dispatch: id-prefix → `GET /v1/{kind}/:id` → `HydratedNode`,
//! plus the version-history fetch helpers shared with `hydra graph diff`
//! (PR 4) and `hydra graph log` (PR 5).
//!
//! Per-kind projection and version-selection logic is exposed as methods on
//! [`HydratedNode`] / [`VersionedNode`] / [`VersionView`] so the dispatch
//! layer stays focused on wiring (id → enum variant → fetch routing).

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use hydra_common::actor_ref::ActorRef;
use hydra_common::api::v1::conversations::Conversation as ApiConversation;
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
// Phase 4b: `Issue.assignee` widened from `Option<String>` to
// `Option<Principal>`, pushing `IssueVersionRecord` over the
// `large_enum_variant` threshold. We box it; the remaining variants are
// kept inline for the simpler match arms.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum HydratedNode {
    Issue(Box<IssueVersionRecord>),
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

    /// Render the hydrated node through the per-kind
    /// `GraphView::view_lN` projection.
    pub fn render(&self, level: VerbosityLevel) -> Value {
        match self {
            HydratedNode::Issue(r) => render_view(&r.issue, level),
            HydratedNode::Patch(r) => render_view(&r.patch, level),
            HydratedNode::Document(r) => render_view(&r.document, level),
            HydratedNode::Conversation(c) => render_view(c, level),
        }
    }
}

fn render_view<T: GraphView>(item: &T, level: VerbosityLevel) -> Value {
    match level {
        VerbosityLevel::L1 => item.view_l1(),
        VerbosityLevel::L2 => item.view_l2(),
        VerbosityLevel::L3 => item.view_l3(),
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
        return Ok(HydratedNode::Issue(Box::new(record)));
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

    /// Iterate over each version in storage order, yielding a typed
    /// [`VersionView`] per entry. Shared between `diff` and `log` so neither
    /// has to match on the variant directly to walk the history vector.
    pub fn iter_views(&self) -> Box<dyn Iterator<Item = VersionView<'_>> + '_> {
        match self {
            VersionedNode::Issue(v) => Box::new(
                v.iter()
                    .enumerate()
                    .map(|(index, record)| VersionView::Issue { record, index }),
            ),
            VersionedNode::Patch(v) => Box::new(
                v.iter()
                    .enumerate()
                    .map(|(index, record)| VersionView::Patch { record, index }),
            ),
            VersionedNode::Document(v) => Box::new(
                v.iter()
                    .enumerate()
                    .map(|(index, record)| VersionView::Document { record, index }),
            ),
            VersionedNode::Conversation(v) => Box::new(
                v.iter()
                    .enumerate()
                    .map(|(index, record)| VersionView::Conversation { record, index }),
            ),
        }
    }

    /// Return the latest version whose timestamp is `<= at`, or `None` if no
    /// such version exists. The versions vector is assumed to be ordered by
    /// ascending `timestamp` (true for all four kinds: server-side stores
    /// append in order; the conversation fold preserves event order).
    pub fn version_at(&self, at: DateTime<Utc>) -> Option<VersionView<'_>> {
        match self {
            VersionedNode::Issue(v) => v
                .iter()
                .enumerate()
                .rev()
                .find(|(_, r)| r.timestamp <= at)
                .map(|(i, r)| VersionView::Issue {
                    record: r,
                    index: i,
                }),
            VersionedNode::Patch(v) => v
                .iter()
                .enumerate()
                .rev()
                .find(|(_, r)| r.timestamp <= at)
                .map(|(i, r)| VersionView::Patch {
                    record: r,
                    index: i,
                }),
            VersionedNode::Document(v) => v
                .iter()
                .enumerate()
                .rev()
                .find(|(_, r)| r.timestamp <= at)
                .map(|(i, r)| VersionView::Document {
                    record: r,
                    index: i,
                }),
            VersionedNode::Conversation(v) => v
                .iter()
                .enumerate()
                .rev()
                .find(|(_, r)| r.timestamp <= at)
                .map(|(i, r)| VersionView::Conversation {
                    record: r,
                    index: i,
                }),
        }
    }
}

/// A typed borrow of a single version within a [`VersionedNode`].
///
/// Carries the per-kind value alongside its version number, timestamp, and
/// vector index, and exposes [`VersionView::render`] for the matching
/// `GraphView::view_lN` projection.
#[derive(Debug, Clone, Copy)]
pub enum VersionView<'a> {
    Issue {
        record: &'a IssueVersionRecord,
        index: usize,
    },
    Patch {
        record: &'a PatchVersionRecord,
        index: usize,
    },
    Document {
        record: &'a DocumentVersionRecord,
        index: usize,
    },
    Conversation {
        record: &'a Versioned<ApiConversation>,
        index: usize,
    },
}

impl<'a> VersionView<'a> {
    /// Version number of the borrowed version.
    pub fn version(&self) -> VersionNumber {
        match self {
            VersionView::Issue { record, .. } => record.version,
            VersionView::Patch { record, .. } => record.version,
            VersionView::Document { record, .. } => record.version,
            VersionView::Conversation { record, .. } => record.version,
        }
    }

    /// Wall-clock timestamp of the borrowed version.
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            VersionView::Issue { record, .. } => record.timestamp,
            VersionView::Patch { record, .. } => record.timestamp,
            VersionView::Document { record, .. } => record.timestamp,
            VersionView::Conversation { record, .. } => record.timestamp,
        }
    }

    /// Index of this version within the underlying history vector.
    pub fn index(&self) -> usize {
        match self {
            VersionView::Issue { index, .. }
            | VersionView::Patch { index, .. }
            | VersionView::Document { index, .. }
            | VersionView::Conversation { index, .. } => *index,
        }
    }

    /// Actor attribution for this version, if recorded. Pre-actor-tracking
    /// versions return `None`.
    pub fn actor(&self) -> Option<&'a ActorRef> {
        match self {
            VersionView::Issue { record, .. } => record.actor.as_ref(),
            VersionView::Patch { record, .. } => record.actor.as_ref(),
            VersionView::Document { record, .. } => record.actor.as_ref(),
            VersionView::Conversation { record, .. } => record.actor.as_ref(),
        }
    }

    /// Project this version through the per-kind `GraphView::view_lN`.
    pub fn render(&self, level: VerbosityLevel) -> Value {
        match self {
            VersionView::Issue { record, .. } => render_view(&record.issue, level),
            VersionView::Patch { record, .. } => render_view(&record.patch, level),
            VersionView::Document { record, .. } => render_view(&record.document, level),
            VersionView::Conversation { record, .. } => render_view(&record.item, level),
        }
    }
}

/// Fetch the full version history of a node from the server.
///
/// `kind` must match the prefix of `id`; this is asserted in debug builds and
/// errored on in release builds. Conversation versions come from
/// `GET /v1/conversations/:id/versions` and are returned by the server as a
/// `Vec<Versioned<Conversation>>` — one row per status transition.
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
            let snapshots = client.get_conversation_versions(&conv_id).await?;
            Ok(VersionedNode::Conversation(snapshots))
        }
    }
}
