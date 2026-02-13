use anyhow::{anyhow, Result};
use bytes::Bytes;
use futures::{stream, Stream, StreamExt};
use metis_common::api::v1::events::{
    EntityEventData, HeartbeatEventData, ResyncEventData, SnapshotEventData, SseEventType,
};
use std::pin::Pin;

/// A parsed SSE event from the /v1/events endpoint.
#[derive(Debug, Clone)]
pub struct SseEvent {
    /// The event type.
    pub event_type: SseEventType,
    /// The sequence ID (from the SSE `id:` field), if present.
    pub id: Option<u64>,
    /// The raw JSON data string.
    pub data: String,
}

impl SseEvent {
    /// Parse the data payload as an entity mutation event.
    pub fn as_entity_event(&self) -> Result<EntityEventData> {
        serde_json::from_str(&self.data).map_err(|e| anyhow!("failed to parse entity event: {e}"))
    }

    /// Parse the data payload as a snapshot event.
    pub fn as_snapshot(&self) -> Result<SnapshotEventData> {
        serde_json::from_str(&self.data).map_err(|e| anyhow!("failed to parse snapshot event: {e}"))
    }

    /// Parse the data payload as a resync event.
    pub fn as_resync(&self) -> Result<ResyncEventData> {
        serde_json::from_str(&self.data).map_err(|e| anyhow!("failed to parse resync event: {e}"))
    }

    /// Parse the data payload as a heartbeat event.
    pub fn as_heartbeat(&self) -> Result<HeartbeatEventData> {
        serde_json::from_str(&self.data)
            .map_err(|e| anyhow!("failed to parse heartbeat event: {e}"))
    }
}

/// A stream of parsed SSE events.
pub type SseEventStream = Pin<Box<dyn Stream<Item = Result<SseEvent>> + Send>>;

type BytesStream = Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>;

/// Parse an SSE byte stream into a stream of typed SseEvents.
///
/// This handles the SSE wire format: blocks separated by blank lines,
/// with `event:`, `id:`, and `data:` fields.
pub(crate) fn parse_sse_event_stream(byte_stream: BytesStream) -> SseEventStream {
    Box::pin(
        stream::unfold(
            (byte_stream, String::new(), false),
            |(mut byte_stream, mut buffer, finished)| async move {
                if finished {
                    return None;
                }

                loop {
                    // Look for a complete event block (terminated by blank line).
                    if let Some((idx, separator_len)) = buffer
                        .find("\n\n")
                        .map(|idx| (idx, 2))
                        .or_else(|| buffer.find("\r\n\r\n").map(|idx| (idx, "\r\n\r\n".len())))
                    {
                        let event_block = buffer[..idx].to_string();
                        buffer.drain(..idx + separator_len);

                        if event_block.trim().is_empty() {
                            continue;
                        }

                        // Skip SSE comments (lines starting with ':')
                        if event_block
                            .lines()
                            .all(|l| l.starts_with(':') || l.is_empty())
                        {
                            continue;
                        }

                        if let Some(event) = parse_event_block(&event_block) {
                            return Some((Ok(event), (byte_stream, buffer, false)));
                        }
                        // Unparseable block — skip it.
                        continue;
                    }

                    match byte_stream.next().await {
                        Some(Ok(chunk)) => {
                            if chunk.is_empty() {
                                continue;
                            }
                            let chunk_text = String::from_utf8_lossy(&chunk);
                            buffer.push_str(&chunk_text);
                        }
                        Some(Err(err)) => {
                            return Some((
                                Err(anyhow!("SSE stream error: {err}")),
                                (byte_stream, buffer, true),
                            ));
                        }
                        None => {
                            // Stream ended. Try to parse any remaining data.
                            if !buffer.trim().is_empty() {
                                if let Some(event) = parse_event_block(&buffer) {
                                    return Some((Ok(event), (byte_stream, String::new(), true)));
                                }
                            }
                            return None;
                        }
                    }
                }
            },
        )
        .fuse(),
    )
}

/// Parse a single SSE event block into an SseEvent.
fn parse_event_block(block: &str) -> Option<SseEvent> {
    let mut event_name = None;
    let mut id_value = None;
    let mut data_lines = Vec::new();

    for line in block.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event_name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("id:") {
            id_value = value.trim().parse::<u64>().ok();
        } else if let Some(value) = line.strip_prefix("data:") {
            let trimmed = value.strip_prefix(' ').unwrap_or(value);
            data_lines.push(trimmed);
        } else if line.starts_with(':') {
            // SSE comment, skip
        }
    }

    if data_lines.is_empty() {
        return None;
    }

    let data = data_lines.join("\n");
    let event_type = match event_name.as_deref() {
        Some(name) => name.parse::<SseEventType>().ok()?,
        None => return None,
    };

    Some(SseEvent {
        event_type,
        id: id_value,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    fn bytes_stream(data: &[u8]) -> BytesStream {
        Box::pin(stream::iter(vec![Ok(Bytes::from(data.to_vec()))]))
    }

    #[tokio::test]
    async fn parses_entity_event() {
        let raw = b"event: job_updated\nid: 5\ndata: {\"entity_type\":\"job\",\"entity_id\":\"t-123\",\"version\":3,\"timestamp\":\"2026-01-01T00:00:00Z\"}\n\n";
        let mut stream = parse_sse_event_stream(bytes_stream(raw));

        let event = stream.next().await.unwrap().unwrap();
        assert_eq!(event.event_type, SseEventType::JobUpdated);
        assert_eq!(event.id, Some(5));

        let entity = event.as_entity_event().unwrap();
        assert_eq!(entity.entity_id, "t-123");
        assert_eq!(entity.entity_type, "job");
        assert_eq!(entity.version, 3);
    }

    #[tokio::test]
    async fn parses_snapshot_event() {
        let raw = b"event: snapshot\nid: 1\ndata: {\"versions\":{\"i-abc\":2}}\n\n";
        let mut stream = parse_sse_event_stream(bytes_stream(raw));

        let event = stream.next().await.unwrap().unwrap();
        assert_eq!(event.event_type, SseEventType::Snapshot);
        let snapshot = event.as_snapshot().unwrap();
        assert_eq!(snapshot.versions.get("i-abc"), Some(&2));
    }

    #[tokio::test]
    async fn parses_resync_event() {
        let raw = b"event: resync\nid: 10\ndata: {\"reason\":\"lagged\",\"current_seq\":10}\n\n";
        let mut stream = parse_sse_event_stream(bytes_stream(raw));

        let event = stream.next().await.unwrap().unwrap();
        assert_eq!(event.event_type, SseEventType::Resync);
        let resync = event.as_resync().unwrap();
        assert_eq!(resync.reason, "lagged");
        assert_eq!(resync.current_seq, 10);
    }

    #[tokio::test]
    async fn parses_heartbeat_event() {
        let raw = b"event: heartbeat\ndata: {\"server_time\":\"2026-01-01T00:00:00Z\"}\n\n";
        let mut stream = parse_sse_event_stream(bytes_stream(raw));

        let event = stream.next().await.unwrap().unwrap();
        assert_eq!(event.event_type, SseEventType::Heartbeat);
        event.as_heartbeat().unwrap();
    }

    #[tokio::test]
    async fn skips_comment_blocks() {
        let raw = b": keep-alive\n\nevent: heartbeat\ndata: {\"server_time\":\"2026-01-01T00:00:00Z\"}\n\n";
        let mut stream = parse_sse_event_stream(bytes_stream(raw));

        let event = stream.next().await.unwrap().unwrap();
        assert_eq!(event.event_type, SseEventType::Heartbeat);
    }

    #[tokio::test]
    async fn handles_multiple_events() {
        let raw = b"event: job_created\nid: 1\ndata: {\"entity_type\":\"job\",\"entity_id\":\"t-1\",\"version\":1,\"timestamp\":\"2026-01-01T00:00:00Z\"}\n\nevent: job_updated\nid: 2\ndata: {\"entity_type\":\"job\",\"entity_id\":\"t-1\",\"version\":2,\"timestamp\":\"2026-01-01T00:00:01Z\"}\n\n";
        let mut stream = parse_sse_event_stream(bytes_stream(raw));

        let e1 = stream.next().await.unwrap().unwrap();
        assert_eq!(e1.event_type, SseEventType::JobCreated);
        assert_eq!(e1.id, Some(1));

        let e2 = stream.next().await.unwrap().unwrap();
        assert_eq!(e2.event_type, SseEventType::JobUpdated);
        assert_eq!(e2.id, Some(2));

        assert!(stream.next().await.is_none());
    }
}
