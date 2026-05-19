//! Hydrated-node dispatch: id-prefix → `GET /v1/{kind}/:id` → `HydratedNode`,
//! plus the `render_view` shim that fans out to per-kind `GraphView::view_lN`.

use anyhow::{anyhow, Result};
use hydra_common::api::v1::conversations::Conversation as ApiConversation;
use hydra_common::documents::DocumentVersionRecord;
use hydra_common::graph::{GraphView, ObjectKind, VerbosityLevel};
use hydra_common::issues::IssueVersionRecord;
use hydra_common::patches::PatchVersionRecord;
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
