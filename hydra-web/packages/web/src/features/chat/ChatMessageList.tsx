import { useEffect, useRef } from "react";
import type { SessionEvent } from "@hydra/api";
import { Avatar } from "@hydra/ui";
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

interface RenderContext {
  agentName: string;
  creator: string;
  userAuthorLabel: string;
}

function renderEvent(event: SessionEvent, index: number, ctx: RenderContext) {
  switch (event.type) {
    case "user_message":
      return (
        <div key={index} className={styles.userRow}>
          <div className={styles.userHead}>
            <Avatar name={ctx.creator} kind="human" size="sm" />
            <span className={styles.msgAuthor}>{ctx.userAuthorLabel}</span>
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
            <Avatar name={ctx.agentName} kind="agent" size="sm" />
            <span className={styles.msgAuthor}>{ctx.agentName}</span>
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
  creator?: string;
  currentUsername?: string | null;
}

export function ChatMessageList({
  events,
  agentName,
  creator,
  currentUsername,
}: ChatMessageListProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const resolvedAgent = agentName || "Agent";
  const resolvedCreator = creator ?? "";
  const isCreatorMe =
    !!resolvedCreator && currentUsername != null && currentUsername === resolvedCreator;
  const userAuthorLabel = isCreatorMe || !resolvedCreator ? "You" : resolvedCreator;
  const renderCtx: RenderContext = {
    agentName: resolvedAgent,
    creator: resolvedCreator,
    userAuthorLabel,
  };

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
        {events.map((event, i) => renderEvent(event, i, renderCtx))}
      </div>
    </div>
  );
}
