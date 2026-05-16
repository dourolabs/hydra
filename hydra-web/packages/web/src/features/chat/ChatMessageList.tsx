import { useEffect, useRef } from "react";
import { MarkdownViewer } from "@hydra/ui";
import type { ConversationEvent } from "@hydra/api";
import { formatRelativeTime, formatTimestamp } from "../../utils/time";
import styles from "./ChatMessageList.module.css";

function SystemEvent({ text, timestamp }: { text: string; timestamp: string }) {
  return (
    <div className={styles.systemEvent} title={formatTimestamp(timestamp)}>
      <span className={styles.systemText}>{text}</span>
    </div>
  );
}

function renderEvent(event: ConversationEvent, index: number, agentName: string) {
  switch (event.type) {
    case "user_message":
      return (
        <div key={index} className={styles.userRow}>
          <div className={styles.userHead}>
            <span className={styles.msgAuthor}>You</span>
            <span className={styles.msgWhen} title={formatTimestamp(event.timestamp)}>
              {formatRelativeTime(event.timestamp)}
            </span>
          </div>
          <div className={styles.userBubble}>{event.content}</div>
        </div>
      );
    case "assistant_message":
      return (
        <div key={index} className={styles.agentRow}>
          <div className={styles.agentHead}>
            <span className={styles.msgAuthor}>{agentName}</span>
            <span className={styles.msgWhen} title={formatTimestamp(event.timestamp)}>
              {formatRelativeTime(event.timestamp)}
            </span>
          </div>
          <div className={styles.agentBody}>
            <MarkdownViewer content={event.content} />
          </div>
        </div>
      );
    case "suspending":
      return (
        <SystemEvent
          key={index}
          text={`Session suspended: ${event.reason}`}
          timestamp={event.timestamp}
        />
      );
    case "resumed":
      return <SystemEvent key={index} text="Session resumed" timestamp={event.timestamp} />;
    case "closed":
      return <SystemEvent key={index} text="Conversation closed" timestamp={event.timestamp} />;
  }
}

interface ChatMessageListProps {
  events: ConversationEvent[];
  agentName?: string | null;
}

export function ChatMessageList({ events, agentName }: ChatMessageListProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const resolvedAgent = agentName || "Agent";

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    container.scrollTo({ top: container.scrollHeight, behavior: "smooth" });
  }, [events.length]);

  if (events.length === 0) {
    return (
      <div ref={containerRef} className={styles.container} data-testid="chat-message-list">
        <div className={styles.empty}>
          <p className={styles.emptyText}>
            No messages yet. Send a message to start the conversation.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div ref={containerRef} className={styles.container} data-testid="chat-message-list">
      <div className={styles.thread}>
        {events.map((event, i) => renderEvent(event, i, resolvedAgent))}
      </div>
    </div>
  );
}
