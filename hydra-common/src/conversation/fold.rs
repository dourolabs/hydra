//! Fold a conversation event stream into a sequence of post-event snapshots.

use crate::api::v1::conversations::{Conversation, ConversationEvent, ConversationStatus};
use crate::versioning::Versioned;
use chrono::{DateTime, Utc};

/// Replays each event in order against the initial snapshot, producing one
/// [`Versioned<Conversation>`] per event.
///
/// Each output entry's `item` is the conversation snapshot *after* applying the
/// corresponding event. The `version`, `timestamp`, `actor`, and
/// `creation_time` of each output entry are taken verbatim from the input
/// `Versioned<ConversationEvent>` at the same index.
///
/// No "v0" pre-event entry is included: the first returned element reflects
/// the post-event snapshot after applying `events[0]`. This matches the
/// design choice that `Store::get_conversation` already exposes the initial
/// (pre-event) snapshot on its own.
///
/// Event semantics applied to the snapshot:
/// - `Resumed` → `status = Active`.
/// - `Suspending` → `status = Idle`.
/// - `Closed` → `status = Closed`.
/// - Every event updates `updated_at` to the event's timestamp.
/// - All other fields (`title`, `agent_name`, `creator`, `session_settings`,
///   `created_at`, `conversation_id`) are inherited from the previous snapshot.
pub fn events_to_versions(
    initial: &Conversation,
    events: &[Versioned<ConversationEvent>],
) -> Vec<Versioned<Conversation>> {
    let mut current = initial.clone();
    let mut out = Vec::with_capacity(events.len());
    for v in events {
        current = apply_event(&current, &v.item);
        out.push(Versioned::with_optional_actor(
            current.clone(),
            v.version,
            v.timestamp,
            v.actor.clone(),
            v.creation_time,
        ));
    }
    out
}

fn apply_event(conv: &Conversation, event: &ConversationEvent) -> Conversation {
    let mut next = conv.clone();
    let (timestamp, status) = event_effect(event);
    next.updated_at = timestamp;
    next.status = status;
    next
}

fn event_effect(event: &ConversationEvent) -> (DateTime<Utc>, ConversationStatus) {
    match event {
        ConversationEvent::Suspending { timestamp, .. } => (*timestamp, ConversationStatus::Idle),
        ConversationEvent::Resumed { timestamp, .. } => (*timestamp, ConversationStatus::Active),
        ConversationEvent::Closed { timestamp } => (*timestamp, ConversationStatus::Closed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::v1::issues::SessionSettings;
    use crate::users::Username;
    use crate::{ActorRef, ConversationId, SessionId};
    use chrono::{Duration, TimeZone};

    fn sample_conversation(created_at: DateTime<Utc>) -> Conversation {
        use crate::api::v1::agents::AgentName;
        Conversation::new(
            ConversationId::new(),
            Some("hello".to_string()),
            Some(AgentName::try_new("swe").unwrap()),
            ConversationStatus::Active,
            Username::from("creator"),
            SessionSettings::default(),
            created_at,
            created_at,
        )
    }

    fn versioned_event(
        event: ConversationEvent,
        version: u64,
        timestamp: DateTime<Utc>,
    ) -> Versioned<ConversationEvent> {
        Versioned::with_actor(event, version, timestamp, ActorRef::test(), timestamp)
    }

    #[test]
    fn closed_event_sets_status_closed() {
        let created_at = Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap();
        let initial = sample_conversation(created_at);
        let ts = created_at + Duration::minutes(5);
        let events = vec![versioned_event(
            ConversationEvent::Closed { timestamp: ts },
            7,
            ts,
        )];
        let out = events_to_versions(&initial, &events);
        assert_eq!(out[0].item.status, ConversationStatus::Closed);
        assert_eq!(out[0].version, 7);
    }

    #[test]
    fn suspending_event_sets_status_idle() {
        let created_at = Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap();
        let initial = sample_conversation(created_at);
        let ts = created_at + Duration::minutes(1);
        let events = vec![versioned_event(
            ConversationEvent::Suspending {
                reason: "idle_timeout".to_string(),
                timestamp: ts,
            },
            1,
            ts,
        )];
        let out = events_to_versions(&initial, &events);
        assert_eq!(out[0].item.status, ConversationStatus::Idle);
    }

    #[test]
    fn resumed_event_sets_status_active() {
        let created_at = Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap();
        let mut initial = sample_conversation(created_at);
        initial.status = ConversationStatus::Closed;
        let ts = created_at + Duration::minutes(2);
        let events = vec![versioned_event(
            ConversationEvent::Resumed {
                session_id: SessionId::new(),
                timestamp: ts,
            },
            1,
            ts,
        )];
        let out = events_to_versions(&initial, &events);
        assert_eq!(out[0].item.status, ConversationStatus::Active);
    }

    #[test]
    fn multiple_events_produce_one_snapshot_per_event_with_monotonic_versions() {
        let created_at = Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap();
        let initial = sample_conversation(created_at);
        let ts1 = created_at + Duration::seconds(10);
        let ts2 = created_at + Duration::seconds(20);
        let ts3 = created_at + Duration::seconds(30);

        let events = vec![
            versioned_event(
                ConversationEvent::Suspending {
                    reason: "idle".to_string(),
                    timestamp: ts1,
                },
                1,
                ts1,
            ),
            versioned_event(
                ConversationEvent::Resumed {
                    session_id: SessionId::new(),
                    timestamp: ts2,
                },
                2,
                ts2,
            ),
            versioned_event(ConversationEvent::Closed { timestamp: ts3 }, 3, ts3),
        ];

        let out = events_to_versions(&initial, &events);
        assert_eq!(out.len(), events.len());

        let versions: Vec<u64> = out.iter().map(|v| v.version).collect();
        assert_eq!(versions, vec![1, 2, 3]);
        for w in versions.windows(2) {
            assert!(w[0] < w[1], "versions must be strictly increasing");
        }

        // Final snapshot reflects the Closed event.
        assert_eq!(out.last().unwrap().item.status, ConversationStatus::Closed);
        // Mid-stream Suspending entry shows Idle.
        assert_eq!(out[0].item.status, ConversationStatus::Idle);
        // Resumed flips back to Active.
        assert_eq!(out[1].item.status, ConversationStatus::Active);
    }

    #[test]
    fn timestamps_and_actors_match_input_events() {
        let created_at = Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap();
        let initial = sample_conversation(created_at);

        let ts1 = created_at + Duration::seconds(1);
        let ts2 = created_at + Duration::seconds(2);
        let event1 = Versioned::with_actor(
            ConversationEvent::Suspending {
                reason: "idle".to_string(),
                timestamp: ts1,
            },
            1,
            ts1,
            ActorRef::test(),
            ts1,
        );
        let event2 = Versioned::new(ConversationEvent::Closed { timestamp: ts2 }, 2, ts2, ts2);

        let out = events_to_versions(&initial, &[event1.clone(), event2.clone()]);
        assert_eq!(out[0].timestamp, event1.timestamp);
        assert_eq!(out[0].actor, event1.actor);
        assert_eq!(out[0].creation_time, event1.creation_time);
        assert_eq!(out[1].timestamp, event2.timestamp);
        assert_eq!(out[1].actor, event2.actor);
        assert_eq!(out[1].creation_time, event2.creation_time);
    }

    #[test]
    fn scrambling_event_order_changes_final_snapshot() {
        let created_at = Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap();
        let initial = sample_conversation(created_at);
        let ts1 = created_at + Duration::seconds(10);
        let ts2 = created_at + Duration::seconds(20);

        let closed = versioned_event(ConversationEvent::Closed { timestamp: ts1 }, 1, ts1);
        let resumed = versioned_event(
            ConversationEvent::Resumed {
                session_id: SessionId::new(),
                timestamp: ts2,
            },
            2,
            ts2,
        );

        let in_order = events_to_versions(&initial, &[closed.clone(), resumed.clone()]);
        let reversed = events_to_versions(&initial, &[resumed, closed]);

        assert_eq!(
            in_order.last().unwrap().item.status,
            ConversationStatus::Active
        );
        assert_eq!(
            reversed.last().unwrap().item.status,
            ConversationStatus::Closed
        );
    }

    #[test]
    fn fold_preserves_creator_title_and_agent() {
        let created_at = Utc.with_ymd_and_hms(2026, 5, 19, 12, 0, 0).unwrap();
        let initial = sample_conversation(created_at);
        let events = vec![versioned_event(
            ConversationEvent::Closed {
                timestamp: created_at + Duration::seconds(5),
            },
            1,
            created_at + Duration::seconds(5),
        )];
        let out = events_to_versions(&initial, &events);
        assert_eq!(out[0].item.title, initial.title);
        assert_eq!(out[0].item.agent_name, initial.agent_name);
        assert_eq!(out[0].item.creator, initial.creator);
        assert_eq!(out[0].item.conversation_id, initial.conversation_id);
    }
}
