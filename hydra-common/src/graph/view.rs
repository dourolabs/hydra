use std::{fmt, str::FromStr};

use serde_json::Value;

use crate::HydraId;

/// The kind of object participating in the knowledge graph.
///
/// Mirrors the variants of `hydra-server`'s `ObjectKind` but lives in
/// `hydra-common` so the `GraphView` trait (consumed by the CLI and other
/// non-server callers) does not need to pull in `hydra-server`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    Issue,
    Patch,
    Document,
    Conversation,
}

impl ObjectKind {
    /// Identify the kind of object addressed by `id` from its prefix
    /// (`i-` / `p-` / `d-` / `c-`). Returns `None` for ids that don't belong
    /// to a graph object kind.
    pub fn from_id(id: &HydraId) -> Option<Self> {
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

    /// Snake-case display string used by JSONL output (`"issue"`, `"patch"`,
    /// `"document"`, `"conversation"`).
    pub const fn as_str(self) -> &'static str {
        match self {
            ObjectKind::Issue => "issue",
            ObjectKind::Patch => "patch",
            ObjectKind::Document => "document",
            ObjectKind::Conversation => "conversation",
        }
    }
}

/// Error returned when a string does not match any [`ObjectKind`] variant.
///
/// The `Display` impl spells out the accepted values so callers (e.g.
/// the `hydra-common::graph::query` DSL parser) can surface it as a hint
/// without duplicating the value list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseObjectKindError;

impl fmt::Display for ParseObjectKindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("expected one of: issue, patch, document, conversation")
    }
}

impl std::error::Error for ParseObjectKindError {}

impl FromStr for ObjectKind {
    type Err = ParseObjectKindError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "issue" => Ok(ObjectKind::Issue),
            "patch" => Ok(ObjectKind::Patch),
            "document" => Ok(ObjectKind::Document),
            "conversation" => Ok(ObjectKind::Conversation),
            _ => Err(ParseObjectKindError),
        }
    }
}

/// Selects the level of detail returned by `GraphView::view_lN`.
///
/// L1 is the terse default (typically title + status); L2 adds the handful of
/// fields agents most often want to see change; L3 is the full struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VerbosityLevel {
    L1,
    L2,
    L3,
}

/// Self-contained per-kind verbosity contract.
///
/// Each graph object kind owns its own impl, co-located with the type
/// definition. `diff` / `log` consumers JSON-diff the output of `view_lN`
/// rather than the raw struct, so this trait is the single point of control
/// over what shows up as a change at each verbosity level.
pub trait GraphView {
    const KIND: ObjectKind;

    fn view_l1(&self) -> Value;
    fn view_l2(&self) -> Value;
    fn view_l3(&self) -> Value;
}
