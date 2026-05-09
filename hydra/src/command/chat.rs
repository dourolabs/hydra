use anyhow::{Context, Result};
use futures::StreamExt;
use hydra_common::api::v1::{
    conversations::{ConversationEvent, CreateConversationRequest, SendMessageRequest},
    events::{EventsQuery, SseEventType},
};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::{client::HydraClientInterface, command::output::CommandContext};

pub async fn run(
    client: &dyn HydraClientInterface,
    prompt: Option<String>,
    agent: Option<String>,
    _context: &CommandContext,
) -> Result<()> {
    match prompt {
        Some(prompt) => run_noninteractive(client, &prompt, agent).await,
        None => run_interactive(client, agent).await,
    }
}

async fn run_noninteractive(
    client: &dyn HydraClientInterface,
    prompt: &str,
    agent: Option<String>,
) -> Result<()> {
    let request = CreateConversationRequest {
        message: prompt.to_string(),
        agent_name: agent,
    };
    let conversation = client
        .create_conversation(&request)
        .await
        .context("failed to create conversation")?;
    let conversation_id = &conversation.conversation_id;

    // Subscribe to SSE events and wait for the assistant response.
    let query = EventsQuery {
        types: Some("conversation_event_created".to_string()),
        ..Default::default()
    };
    let mut event_stream = client
        .subscribe_events(&query, None)
        .await
        .context("failed to subscribe to events")?;

    let event_loop = async {
        while let Some(event_result) = event_stream.next().await {
            let sse_event = event_result.context("SSE stream error")?;
            if sse_event.event_type != SseEventType::ConversationEventCreated {
                continue;
            }
            let entity = sse_event
                .as_entity_event()
                .context("failed to parse entity event")?;
            if !entity.entity_id.starts_with(&conversation_id.to_string()) {
                continue;
            }

            // Parse the conversation event from the entity payload.
            if let Some(entity_value) = &entity.entity {
                if let Ok(conv_event) =
                    serde_json::from_value::<ConversationEvent>(entity_value.clone())
                {
                    match &conv_event {
                        ConversationEvent::AssistantMessage { content, .. } => {
                            println!("{content}");
                            break;
                        }
                        ConversationEvent::Closed { .. } => break,
                        _ => {}
                    }
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    };

    tokio::select! {
        result = event_loop => { result?; }
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\nInterrupted.");
        }
    }

    // Best-effort close.
    let _ = client.close_conversation(conversation_id).await;
    Ok(())
}

async fn run_interactive(client: &dyn HydraClientInterface, agent: Option<String>) -> Result<()> {
    eprintln!("Starting interactive chat session. Press Ctrl+C or Ctrl+D to exit.");
    eprint!("> ");

    // Read the first message from stdin.
    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    let first_message = match lines.next_line().await? {
        Some(line) if !line.trim().is_empty() => line,
        _ => {
            eprintln!("No input received. Exiting.");
            return Ok(());
        }
    };

    let request = CreateConversationRequest {
        message: first_message,
        agent_name: agent,
    };
    let conversation = client
        .create_conversation(&request)
        .await
        .context("failed to create conversation")?;
    let conversation_id = conversation.conversation_id.clone();

    // Subscribe to SSE events for this conversation.
    let query = EventsQuery {
        types: Some("conversation_event_created".to_string()),
        ..Default::default()
    };
    let mut event_stream = client
        .subscribe_events(&query, None)
        .await
        .context("failed to subscribe to events")?;

    // REPL loop: wait for assistant response, then read next user input.
    let repl_loop = async {
        loop {
            // Wait for assistant response.
            let mut got_response = false;
            while let Some(event_result) = event_stream.next().await {
                let sse_event = match event_result {
                    Ok(e) => e,
                    Err(err) => {
                        eprintln!("SSE error: {err}");
                        break;
                    }
                };
                if sse_event.event_type != SseEventType::ConversationEventCreated {
                    continue;
                }
                let entity = match sse_event.as_entity_event() {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                if !entity.entity_id.starts_with(&conversation_id.to_string()) {
                    continue;
                }

                if let Some(entity_value) = &entity.entity {
                    if let Ok(conv_event) =
                        serde_json::from_value::<ConversationEvent>(entity_value.clone())
                    {
                        match &conv_event {
                            ConversationEvent::AssistantMessage { content, .. } => {
                                println!("{content}");
                                got_response = true;
                                break;
                            }
                            ConversationEvent::Closed { .. } => {
                                eprintln!("Conversation closed by server.");
                                return Ok(());
                            }
                            ConversationEvent::Suspending { reason, .. } => {
                                eprintln!("Session suspending: {reason}");
                            }
                            _ => {}
                        }
                    }
                }
            }

            if !got_response {
                eprintln!("Event stream ended.");
                break;
            }

            // Read next user input.
            eprint!("> ");
            let next_message = match lines.next_line().await? {
                Some(line) if !line.trim().is_empty() => line,
                _ => break, // EOF or empty line on Ctrl+D
            };

            let send_request = SendMessageRequest {
                content: next_message,
            };
            client
                .send_message(&conversation_id, &send_request)
                .await
                .context("failed to send message")?;
        }
        Ok::<(), anyhow::Error>(())
    };

    tokio::select! {
        result = repl_loop => { result?; }
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\nInterrupted.");
        }
    }

    // Best-effort close on exit.
    let _ = client.close_conversation(&conversation_id).await;
    eprintln!("Chat session ended.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hydra_common::api::v1::conversations::ConversationEvent;

    #[test]
    fn create_conversation_request_serializes_correctly() {
        let request = CreateConversationRequest {
            message: "hello".to_string(),
            agent_name: Some("test-agent".to_string()),
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["message"], "hello");
        assert_eq!(json["agent_name"], "test-agent");
    }

    #[test]
    fn create_conversation_request_without_agent() {
        let request = CreateConversationRequest {
            message: "hello".to_string(),
            agent_name: None,
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"message\":\"hello\""));
        assert!(!json.contains("agent_name"));
    }

    #[test]
    fn send_message_request_serializes_correctly() {
        let request = SendMessageRequest {
            content: "test message".to_string(),
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["content"], "test message");
    }

    #[test]
    fn assistant_message_event_deserializes() {
        let json = serde_json::json!({
            "type": "assistant_message",
            "content": "Hello! How can I help?",
            "timestamp": "2026-01-01T00:00:00Z"
        });
        let event: ConversationEvent = serde_json::from_value(json).unwrap();
        match event {
            ConversationEvent::AssistantMessage { content, .. } => {
                assert_eq!(content, "Hello! How can I help?");
            }
            _ => panic!("expected AssistantMessage"),
        }
    }

    #[test]
    fn closed_event_deserializes() {
        let json = serde_json::json!({
            "type": "closed",
            "timestamp": "2026-01-01T00:00:00Z"
        });
        let event: ConversationEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(event, ConversationEvent::Closed { .. }));
    }
}
