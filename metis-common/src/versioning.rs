use serde::{Deserialize, Serialize};

/// Monotonic version number associated with stored objects.
pub type VersionNumber = u64;

/// Pairs a value with its version number.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Versioned<T> {
    pub item: T,
    pub version: VersionNumber,
}

impl<T> Versioned<T> {
    pub const fn new(item: T, version: VersionNumber) -> Self {
        Self { item, version }
    }
}
