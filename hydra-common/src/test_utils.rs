pub mod status {
    use crate::api::v1::projects::{StatusDefinition, StatusKey};

    /// Build a [`StatusKey`] from a `&str` known to be a well-formed slug.
    /// Panics on malformed input — intended for test code only.
    pub fn status(key: &str) -> StatusKey {
        StatusKey::try_new(key).expect("test status key is well-formed")
    }

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
