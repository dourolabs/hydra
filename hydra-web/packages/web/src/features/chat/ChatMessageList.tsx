import { useEffect, useRef } from "react";
import type { SessionEvent } from "@hydra/api";
import { Avatar } from "@hydra/ui";
import { Markdown } from "../../components/Markdown";
import { formatTimestamp } from "../../utils/time";
import { AgoTime } from "../../components/Runtime/Runtime";
import { MessageReferencesPreview } from "./MessageReferencesPreview";
import { ChatActivityLine } from "./ChatActivityLine";
import { SystemEventBubble } from "./SystemEventBubble";
import type { ActivityRun } from "./deriveActivitySteps";
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
          <div className={styles.referencesSlot}>
            <MessageReferencesPreview content={event.content} />
          </div>
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
          <MessageReferencesPreview content={event.content} />
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
      return <SystemEvent key={index} text="Session ended" timestamp={event.timestamp} />;
    case "system_event":
      return <SystemEventBubble key={index} kind={event.kind} timestamp={event.timestamp} />;
    case "tool_use":
    case "unknown":
      return null;
    default: {
      // Exhaustiveness guard — a new `SessionEvent` variant must be handled
      // here or the call site will surface a TypeScript error rather than
      // silently dropping the event from the transcript.
      const _exhaustive: never = event;
      void _exhaustive;
      return null;
    }
  }
}

interface ChatMessageListProps {
  events: SessionEvent[];
  agentName?: string | null;
  creator?: string;
  currentUsername?: string | null;
  /**
   * Derived activity run, rendered as the trailing transcript item when an
   * activity is live or just settled. Hidden when there's nothing to show.
   */
  activity?: ActivityRun;
}

/** Whether the activity line should occupy a transcript slot. */
function shouldRenderActivity(activity: ActivityRun | undefined): boolean {
  if (!activity) return false;
  return activity.current !== null || activity.steps.length > 0;
}

export function ChatMessageList({
  events,
  agentName,
  creator,
  currentUsername,
  activity,
}: ChatMessageListProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const threadRef = useRef<HTMLDivElement>(null);
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

  // The activity line appears/disappears between events, and that transition
  // needs to participate in the same scroll-to-bottom behaviour as a new
  // message — otherwise the newly rendered indicator can land behind the
  // input. Include its presence in the effect's dep key.
  const activityVisible = shouldRenderActivity(activity);

  useEffect(() => {
    const container = containerRef.current;
    const thread = threadRef.current;
    if (!container) return;
    container.scrollTo({ top: container.scrollHeight, behavior: "smooth" });

    // Preview cards for `[[id]]` references fetch their target asynchronously
    // and paint after the initial scroll-to-bottom — without this follow-up
    // the trailing card(s) can land below the input box. Re-pin the bottom
    // while the new message's content settles, and stop as soon as the user
    // intentionally scrolls away.
    if (!thread || typeof ResizeObserver === "undefined") return;
    let following = true;
    // Per spec, ResizeObserver fires once on observe with the current size.
    // Capture the current height so that initial fire is a no-op and the
    // smooth-scroll above is not pre-empted by an immediate auto-scroll.
    let lastHeight = thread.getBoundingClientRect().height;
    const observer = new ResizeObserver((entries) => {
      if (!following) return;
      const newHeight = entries[0]?.contentRect.height ?? thread.scrollHeight;
      if (newHeight <= lastHeight) return;
      lastHeight = newHeight;
      container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
    });
    observer.observe(thread);
    const stopFollowing = () => {
      following = false;
    };
    const stopTimer = window.setTimeout(stopFollowing, 1500);
    container.addEventListener("wheel", stopFollowing, { passive: true });
    container.addEventListener("touchmove", stopFollowing, { passive: true });
    return () => {
      observer.disconnect();
      window.clearTimeout(stopTimer);
      container.removeEventListener("wheel", stopFollowing);
      container.removeEventListener("touchmove", stopFollowing);
    };
  }, [events.length, activityVisible]);

  if (events.length === 0 && !activityVisible) {
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
      <div ref={threadRef} className={styles.thread}>
        {events.map((event, i) => renderEvent(event, i, renderCtx))}
        {activityVisible && activity ? (
          <div className={styles.activitySlot}>
            <ChatActivityLine run={activity} />
          </div>
        ) : null}
      </div>
    </div>
  );
}
