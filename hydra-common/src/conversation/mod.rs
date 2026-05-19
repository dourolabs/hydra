//! Shared helpers for working with conversations on the client side.
//!
//! See [`fold::events_to_versions`] for the canonical event-stream-to-snapshot
//! fold used by both the data layer (`Store::get_conversation_versions`) and
//! CLI consumers that read conversation history from
//! `GET /v1/conversations/:id/events` and fold client-side.

pub mod fold;

pub use fold::events_to_versions;
