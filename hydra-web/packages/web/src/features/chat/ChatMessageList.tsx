import { useEffect, useRef } from "react";
import type { SessionEvent } from "@hydra/api";
import { Markdown } from "../../components/Markdown";
import { formatTimestamp } from "../../utils/time";
import { AgoTime } from "../../components/Runtime/Runtime";
import styles from "./ChatMessageList.module.css";

function SystemEvent({ text, timestamp }: { text: string; timestamp: string }) {
  return (
    <div className={styles.systemEvent} title={formatTimestamp(timestamp)}>
      <span className={styles.systemText}>{text}</span>
    </div>
  );
}

function renderEvent(event: SessionEvent, index: number, agentName: string) {
  switch (event.type) {
    case "user_message":
      return (
        <div key={index} className={styles.userRow}>
          <div className={styles.userHead}>
            <span className={styles.msgAuthor}>You</span>
            <span className={styles.msgWhen} title={formatTimestamp(event.timestamp)}>
              <AgoTime iso={event.timestamp} />
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
              <AgoTime iso={event.timestamp} />
            </span>
          </div>
          <div className={styles.agentBody}>
            <Markdown content={event.content} />
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
    case "tool_use":
    case "unknown":
      return null;
  }
}

interface ChatMessageListProps {
  events: SessionEvent[];
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
      <div
        ref={containerRef}
        className={styles.container}
        data-testid="chat-message-list"
        data-transcript-source="session_events"
      >
        <div className={styles.empty}>
          <p className={styles.emptyText}>
            No messages yet. Send a message to start the conversation.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      className={styles.container}
      data-testid="chat-message-list"
      data-transcript-source="session_events"
    >
      <div className={styles.thread}>
        {events.map((event, i) => renderEvent(event, i, resolvedAgent))}
      </div>
    </div>
  );
}
