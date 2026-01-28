use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Monotonic version number associated with stored objects.
pub type VersionNumber = u64;

/// Pairs a value with its version number.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Versioned<T> {
    pub item: T,
    pub version: VersionNumber,
    /// Timestamp when this version was recorded.
    pub timestamp: DateTime<Utc>,
}

impl<T> Versioned<T> {
    pub fn new(item: T, version: VersionNumber, timestamp: DateTime<Utc>) -> Self {
        Self {
            item,
            version,
            timestamp,
        }
    }
}
