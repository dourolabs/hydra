//! `Timeout`: durations with an explicit "infinite" variant.
//!
//! Used wherever a caller might want "wait forever" rather than "fall back
//! to the configured default" — currently `SessionSettings.idle_timeout`
//! and `SessionMode::Interactive.idle_timeout`.
//!
//! Internally-tagged so JSON is
//! `{"kind":"seconds","value":600}` / `{"kind":"infinite"}`. The tagged
//! form disambiguates cleanly against the rest of `SessionSettings`
//! (which is the only other place these flow through serialized state).

use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;

/// A user-facing timeout: either a positive whole number of seconds, or
/// an explicit `Infinite` (meaning "never elapse"). `NonZeroU64` rules
/// out `Timeout::Seconds(0)` at the type level — callers wanting
/// "infinite" must say so explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Timeout {
    Seconds { value: NonZeroU64 },
    Infinite,
}

impl Timeout {
    /// Construct a `Seconds` variant. Returns `None` if `value` is zero.
    pub fn seconds(value: u64) -> Option<Self> {
        NonZeroU64::new(value).map(|value| Self::Seconds { value })
    }
}
