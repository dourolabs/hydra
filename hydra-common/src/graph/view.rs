use serde_json::Value;

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
