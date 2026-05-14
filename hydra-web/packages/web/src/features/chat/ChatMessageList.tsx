import { useEffect, useRef } from "react";
import { MarkdownViewer, Tooltip } from "@hydra/ui";
import type { ConversationEvent } from "@hydra/api";
import { formatTimestamp } from "../../utils/time";
import styles from "./ChatMessageList.module.css";

function SystemEvent({ text, timestamp }: { text: string; timestamp: string }) {
  return (
    <div className={styles.systemEvent}>
      <Tooltip content={formatTimestamp(timestamp)} trigger="hover-or-click">
        <span className={styles.systemText}>{text}</span>
      </Tooltip>
    </div>
  );
}

function renderEvent(event: ConversationEvent, index: number) {
  switch (event.type) {
    case "user_message":
      return (
        <div key={index} className={styles.messageRow}>
          <Tooltip
            content={formatTimestamp(event.timestamp)}
            trigger="hover-or-click"
            position="top"
            className={styles.userBubbleWrapper}
          >
            <div className={styles.userBubble}>
              <div className={styles.bubbleContent}>{event.content}</div>
            </div>
          </Tooltip>
        </div>
      );
    case "assistant_message":
      return (
        <div key={index} className={styles.messageRow}>
          <Tooltip
            content={formatTimestamp(event.timestamp)}
            trigger="hover-or-click"
            position="top"
            className={styles.assistantContent}
          >
            <MarkdownViewer content={event.content} className={styles.markdown} />
          </Tooltip>
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
      return (
        <SystemEvent key={index} text="Session resumed" timestamp={event.timestamp} />
      );
    case "closed":
      return (
        <SystemEvent key={index} text="Conversation closed" timestamp={event.timestamp} />
      );
  }
}

interface ChatMessageListProps {
  events: ConversationEvent[];
}

export function ChatMessageList({ events }: ChatMessageListProps) {
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [events.length]);

  if (events.length === 0) {
    return (
      <div className={styles.container}>
        <div className={styles.empty}>
          <p className={styles.emptyText}>No messages yet. Send a message to start the conversation.</p>
        </div>
      </div>
    );
  }

  return (
    <div className={styles.container}>
      {events.map((event, i) => renderEvent(event, i))}
      <div ref={bottomRef} />
    </div>
  );
}
