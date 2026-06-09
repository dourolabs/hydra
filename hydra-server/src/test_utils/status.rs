use hydra_common::api::v1::projects::{StatusDefinition, StatusKey};

/// Synthesize a neutral [`StatusDefinition`] for tests that need an
/// API-shape `Issue` but don't care about the resolved display props
/// (label / color / dependency flags). Mirrors `hydra::test_utils::status::make_status_def`
/// so the two crates' test fixtures share an identical shape.
pub fn make_status_def(key: StatusKey) -> StatusDefinition {
    StatusDefinition::new(
        key,
        String::new(),
        "#888888".parse().expect("well-formed default rgb"),
        false,
        false,
        false,
        None,
    )
}
