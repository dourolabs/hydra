use chrono::{DateTime, Utc};
use metis_common::{DocumentId, PatchId, TaskId, issues::IssueId};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::broadcast;

/// Events emitted when server-side entities are mutated.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum ServerEvent {
    IssueCreated {
        seq: u64,
        issue_id: IssueId,
        timestamp: DateTime<Utc>,
    },
    IssueUpdated {
        seq: u64,
        issue_id: IssueId,
        timestamp: DateTime<Utc>,
    },
    IssueDeleted {
        seq: u64,
        issue_id: IssueId,
        timestamp: DateTime<Utc>,
    },
    PatchCreated {
        seq: u64,
        patch_id: PatchId,
        timestamp: DateTime<Utc>,
    },
    PatchUpdated {
        seq: u64,
        patch_id: PatchId,
        timestamp: DateTime<Utc>,
    },
    PatchDeleted {
        seq: u64,
        patch_id: PatchId,
        timestamp: DateTime<Utc>,
    },
    JobCreated {
        seq: u64,
        task_id: TaskId,
        timestamp: DateTime<Utc>,
    },
    JobUpdated {
        seq: u64,
        task_id: TaskId,
        timestamp: DateTime<Utc>,
    },
    DocumentCreated {
        seq: u64,
        document_id: DocumentId,
        timestamp: DateTime<Utc>,
    },
    DocumentUpdated {
        seq: u64,
        document_id: DocumentId,
        timestamp: DateTime<Utc>,
    },
    DocumentDeleted {
        seq: u64,
        document_id: DocumentId,
        timestamp: DateTime<Utc>,
    },
}

impl ServerEvent {
    /// Returns the monotonic sequence number for this event.
    pub fn seq(&self) -> u64 {
        match self {
            ServerEvent::IssueCreated { seq, .. }
            | ServerEvent::IssueUpdated { seq, .. }
            | ServerEvent::IssueDeleted { seq, .. }
            | ServerEvent::PatchCreated { seq, .. }
            | ServerEvent::PatchUpdated { seq, .. }
            | ServerEvent::PatchDeleted { seq, .. }
            | ServerEvent::JobCreated { seq, .. }
            | ServerEvent::JobUpdated { seq, .. }
            | ServerEvent::DocumentCreated { seq, .. }
            | ServerEvent::DocumentUpdated { seq, .. }
            | ServerEvent::DocumentDeleted { seq, .. } => *seq,
        }
    }
}

const DEFAULT_BUFFER_SIZE: usize = 1024;

/// Broadcast-based event bus for notifying subscribers of entity mutations.
pub struct EventBus {
    sender: broadcast::Sender<ServerEvent>,
    next_seq: AtomicU64,
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(DEFAULT_BUFFER_SIZE);
        Self {
            sender,
            next_seq: AtomicU64::new(1),
        }
    }

    /// Returns a new receiver that will get all future events.
    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.sender.subscribe()
    }

    /// Allocates the next monotonic sequence number.
    fn next_seq(&self) -> u64 {
        self.next_seq.fetch_add(1, Ordering::Relaxed)
    }

    /// Sends an event on the bus. If there are no active receivers the event is
    /// silently dropped (this is normal during startup or when no SSE clients
    /// are connected).
    fn send(&self, event: ServerEvent) {
        let _ = self.sender.send(event);
    }

    pub fn emit_issue_created(&self, issue_id: IssueId) {
        self.send(ServerEvent::IssueCreated {
            seq: self.next_seq(),
            issue_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_issue_updated(&self, issue_id: IssueId) {
        self.send(ServerEvent::IssueUpdated {
            seq: self.next_seq(),
            issue_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_issue_deleted(&self, issue_id: IssueId) {
        self.send(ServerEvent::IssueDeleted {
            seq: self.next_seq(),
            issue_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_patch_created(&self, patch_id: PatchId) {
        self.send(ServerEvent::PatchCreated {
            seq: self.next_seq(),
            patch_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_patch_updated(&self, patch_id: PatchId) {
        self.send(ServerEvent::PatchUpdated {
            seq: self.next_seq(),
            patch_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_patch_deleted(&self, patch_id: PatchId) {
        self.send(ServerEvent::PatchDeleted {
            seq: self.next_seq(),
            patch_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_job_created(&self, task_id: TaskId) {
        self.send(ServerEvent::JobCreated {
            seq: self.next_seq(),
            task_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_job_updated(&self, task_id: TaskId) {
        self.send(ServerEvent::JobUpdated {
            seq: self.next_seq(),
            task_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_document_created(&self, document_id: DocumentId) {
        self.send(ServerEvent::DocumentCreated {
            seq: self.next_seq(),
            document_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_document_updated(&self, document_id: DocumentId) {
        self.send(ServerEvent::DocumentUpdated {
            seq: self.next_seq(),
            document_id,
            timestamp: Utc::now(),
        });
    }

    pub fn emit_document_deleted(&self, document_id: DocumentId) {
        self.send(ServerEvent::DocumentDeleted {
            seq: self.next_seq(),
            document_id,
            timestamp: Utc::now(),
        });
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seq_numbers_are_monotonically_increasing() {
        let bus = EventBus::new();
        let s1 = bus.next_seq();
        let s2 = bus.next_seq();
        let s3 = bus.next_seq();
        assert_eq!(s1, 1);
        assert_eq!(s2, 2);
        assert_eq!(s3, 3);
    }

    #[tokio::test]
    async fn subscribe_receives_emitted_events() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        let issue_id = IssueId::new();
        bus.emit_issue_created(issue_id.clone());

        let event = rx.recv().await.expect("should receive event");
        assert_eq!(event.seq(), 1);
        match event {
            ServerEvent::IssueCreated {
                issue_id: id, seq, ..
            } => {
                assert_eq!(id, issue_id);
                assert_eq!(seq, 1);
            }
            other => panic!("expected IssueCreated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn events_arrive_in_order() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        let issue_id = IssueId::new();
        bus.emit_issue_created(issue_id.clone());
        bus.emit_issue_updated(issue_id);

        let e1 = rx.recv().await.unwrap();
        let e2 = rx.recv().await.unwrap();
        assert!(e1.seq() < e2.seq());
        assert!(matches!(e1, ServerEvent::IssueCreated { .. }));
        assert!(matches!(e2, ServerEvent::IssueUpdated { .. }));
    }
}
