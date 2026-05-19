//! Hydrated-node dispatch: id-prefix → `GET /v1/{kind}/:id` → `HydratedNode`,
//! plus the `render_view` shim that fans out to per-kind `GraphView::view_lN`.
//!
//! Also exposes [`VersionedNode`] and [`fetch_versions`], the per-kind
//! version-fetch helpers used by `hydra graph diff` (and, when it lands,
//! `hydra graph log`). For issues / patches / documents these wrap the
//! existing `GET /v1/{kind}/:id/versions` endpoint; for conversations they
//! fetch `GET /v1/conversations/:id` + `GET /v1/conversations/:id/events` and
//! fold the events client-side via
//! [`hydra_common::conversation::fold::events_to_versions`].

use anyhow::{anyhow, Context, Result};
use hydra_common::api::v1::conversations::{
    Conversation as ApiConversation, ConversationEvent as ApiConversationEvent,
};
use hydra_common::conversation::fold::events_to_versions;
use hydra_common::documents::{Document, DocumentVersionRecord};
use hydra_common::graph::{GraphView, ObjectKind, VerbosityLevel};
use hydra_common::issues::{Issue, IssueVersionRecord};
use hydra_common::patches::{Patch, PatchVersionRecord};
use hydra_common::versioning::Versioned;
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

/// Per-kind version history for a single node.
///
/// Each variant carries the chronologically-ordered version sequence for one
/// node. Constructed by [`fetch_versions`], consumed by `hydra graph diff`
/// (and, in PR 5, `hydra graph log`).
#[derive(Debug, Clone)]
pub enum VersionedNode {
    Issue(Vec<Versioned<Issue>>),
    Patch(Vec<Versioned<Patch>>),
    Document(Vec<Versioned<Document>>),
    Conversation(Vec<Versioned<ApiConversation>>),
}

impl VersionedNode {
    pub fn kind(&self) -> ObjectKind {
        match self {
            VersionedNode::Issue(_) => ObjectKind::Issue,
            VersionedNode::Patch(_) => ObjectKind::Patch,
            VersionedNode::Document(_) => ObjectKind::Document,
            VersionedNode::Conversation(_) => ObjectKind::Conversation,
        }
    }

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
}

/// Fetch the full version history of a single node by id-prefix dispatch.
///
/// Issue / patch / document: one `GET /v1/{kind}/:id/versions` call.
/// Conversation: one `GET /v1/conversations/:id` (for the creation snapshot)
/// plus one `GET /v1/conversations/:id/events`, folded client-side into
/// `Vec<Versioned<Conversation>>` via
/// [`hydra_common::conversation::fold::events_to_versions`]. No new HTTP route
/// is involved.
pub async fn fetch_versions(
    client: &dyn HydraClientInterface,
    id: &HydraId,
) -> Result<VersionedNode> {
    if let Some(issue_id) = id.as_issue_id() {
        let response = client
            .list_issue_versions(&issue_id)
            .await
            .with_context(|| format!("failed to list issue versions for {issue_id}"))?;
        let versions = response
            .versions
            .into_iter()
            .map(|r| {
                Versioned::with_optional_actor(
                    r.issue,
                    r.version,
                    r.timestamp,
                    r.actor,
                    r.creation_time,
                )
            })
            .collect();
        return Ok(VersionedNode::Issue(versions));
    }
    if let Some(patch_id) = id.as_patch_id() {
        let response = client
            .list_patch_versions(&patch_id)
            .await
            .with_context(|| format!("failed to list patch versions for {patch_id}"))?;
        let versions = response
            .versions
            .into_iter()
            .map(|r| {
                Versioned::with_optional_actor(
                    r.patch,
                    r.version,
                    r.timestamp,
                    r.actor,
                    r.creation_time,
                )
            })
            .collect();
        return Ok(VersionedNode::Patch(versions));
    }
    if let Some(doc_id) = id.as_document_id() {
        let response = client
            .list_document_versions(&doc_id)
            .await
            .with_context(|| format!("failed to list document versions for {doc_id}"))?;
        let versions = response
            .versions
            .into_iter()
            .map(|r| {
                Versioned::with_optional_actor(
                    r.document,
                    r.version,
                    r.timestamp,
                    r.actor,
                    r.creation_time,
                )
            })
            .collect();
        return Ok(VersionedNode::Document(versions));
    }
    if let Some(conv_id) = id.as_conversation_id() {
        // Per the design doc, conversation versions are computed entirely
        // client-side: fetch the creation snapshot + the event stream and fold
        // via the shared helper from PR 2. No /v1/conversations/:id/versions
        // HTTP route is involved.
        let (initial, events) = tokio::try_join!(
            client.get_conversation(&conv_id),
            client.get_conversation_events(&conv_id),
        )
        .with_context(|| format!("failed to fetch conversation history for {conv_id}"))?;
        let versioned_events = synthesize_versioned_events(events);
        let folded = events_to_versions(&initial, &versioned_events);
        return Ok(VersionedNode::Conversation(folded));
    }
    Err(anyhow!(
        "id '{id}' does not belong to a graph object kind (expected i-/p-/d-/c- prefix)"
    ))
}

/// Wrap a flat conversation event stream into `Versioned<ConversationEvent>`
/// values so it can be fed to
/// [`hydra_common::conversation::fold::events_to_versions`].
///
/// The HTTP endpoint `GET /v1/conversations/:id/events` returns
/// `Vec<ConversationEvent>` today — the `Versioned` envelope (version number,
/// per-event actor, creation time) is dropped on the wire. We rebuild it
/// client-side using the order-determined index as the version number and the
/// event's own embedded timestamp for both `timestamp` and `creation_time`.
/// `actor` is left as `None` since the HTTP payload does not carry it. The
/// reconstructed envelope is good enough for [`events_to_versions`], whose
/// output is in turn what `diff` / `log` consume.
fn synthesize_versioned_events(
    events: Vec<ApiConversationEvent>,
) -> Vec<Versioned<ApiConversationEvent>> {
    events
        .into_iter()
        .enumerate()
        .map(|(idx, event)| {
            let ts = event_timestamp(&event);
            Versioned::new(event, (idx as u64) + 1, ts, ts)
        })
        .collect()
}

fn event_timestamp(event: &ApiConversationEvent) -> chrono::DateTime<chrono::Utc> {
    match event {
        ApiConversationEvent::UserMessage { timestamp, .. }
        | ApiConversationEvent::AssistantMessage { timestamp, .. }
        | ApiConversationEvent::Suspending { timestamp, .. }
        | ApiConversationEvent::Resumed { timestamp, .. }
        | ApiConversationEvent::Closed { timestamp } => *timestamp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono::Utc;

    #[test]
    fn synthesize_versioned_events_assigns_monotonic_versions_from_one() {
        let ts1 = Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap();
        let ts2 = Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 5).unwrap();
        let events = vec![
            ApiConversationEvent::UserMessage {
                content: "hi".to_string(),
                timestamp: ts1,
            },
            ApiConversationEvent::Closed { timestamp: ts2 },
        ];
        let versioned = synthesize_versioned_events(events);
        assert_eq!(versioned.len(), 2);
        assert_eq!(versioned[0].version, 1);
        assert_eq!(versioned[0].timestamp, ts1);
        assert_eq!(versioned[0].creation_time, ts1);
        assert!(versioned[0].actor.is_none());
        assert_eq!(versioned[1].version, 2);
        assert_eq!(versioned[1].timestamp, ts2);
    }
}
