pub mod status {
    use crate::api::v1::projects::{StatusDefinition, StatusKey};

    /// Neutral `StatusDefinition` for tests that don't care about display
    /// props (empty label, neutral grey, dependency flags off).
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
}
