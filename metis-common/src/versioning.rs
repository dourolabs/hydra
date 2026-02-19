use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::actor_ref::ActorRef;

/// Monotonic version number associated with stored objects.
pub type VersionNumber = u64;

/// A version number that can be positive (exact version) or negative (offset
/// from the latest version).
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct RelativeVersionNumber(i64);

impl RelativeVersionNumber {
    pub fn new(value: i64) -> Self {
        Self(value)
    }

    pub fn as_i64(self) -> i64 {
        self.0
    }
}

impl fmt::Display for RelativeVersionNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Pairs a value with its version number.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Versioned<T> {
    pub item: T,
    pub version: VersionNumber,
    /// Timestamp when this version was recorded.
    pub timestamp: DateTime<Utc>,
    /// The actor who performed this mutation.
    /// `None` for historical versions that predate actor tracking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ActorRef>,
}

impl<T> Versioned<T> {
    pub fn new(item: T, version: VersionNumber, timestamp: DateTime<Utc>) -> Self {
        Self {
            item,
            version,
            timestamp,
            actor: None,
        }
    }

    pub fn with_actor(
        item: T,
        version: VersionNumber,
        timestamp: DateTime<Utc>,
        actor: ActorRef,
    ) -> Self {
        Self {
            item,
            version,
            timestamp,
            actor: Some(actor),
        }
    }

    pub fn with_optional_actor(
        item: T,
        version: VersionNumber,
        timestamp: DateTime<Utc>,
        actor: Option<ActorRef>,
    ) -> Self {
        Self {
            item,
            version,
            timestamp,
            actor,
        }
    }
}
