import { Spinner } from "@hydra/ui";
import type { SessionEvent } from "@hydra/api";
import { Link } from "react-router-dom";
import { formatTimestamp } from "../../utils/time";
import { Markdown } from "../../components/Markdown";
import { ApiError } from "../../api/client";
import { useSessionEvents } from "./useSessionEvents";
import styles from "./SessionEventsView.module.css";

interface SessionEventsViewProps {
  sessionId: string;
}

interface RenderedEvent {
  /** Short uppercase tag rendered as a badge to the left of the row. */
  kind: string;
  /** CSS modifier class added to `.row` for per-variant color accents. */
  modifier?: string;
  /** Optional one-line subtitle (e.g. tool name, suspend reason). */
  subtitle?: React.ReactNode;
  /** Main payload — string for plain text, ReactNode for richer content. */
  body?: React.ReactNode;
}

/**
 * Map a `SessionEvent` to its display pieces. First-cut rendering per
 * [[i-pddbxzci]] — surface the fields a human inspecting the log would
 * want to see (kind, timestamp, main payload) without polishing.
 */
function describeEvent(event: SessionEvent): RenderedEvent {
  switch (event.type) {
    case "user_message":
      return {
        kind: "USER",
        modifier: styles.rowUser,
        body: <pre className={styles.text}>{event.content}</pre>,
      };
    case "assistant_message":
      return {
        kind: "ASSISTANT",
        modifier: styles.rowAssistant,
        body: (
          <div className={styles.markdown}>
            <Markdown content={event.content} />
          </div>
        ),
      };
    case "tool_use":
      return {
        kind: "TOOL",
        modifier: styles.rowTool,
        subtitle: <code className={styles.code}>{event.tool_name}</code>,
        body: (
          <pre className={styles.json}>
            {JSON.stringify(event.payload, null, 2)}
          </pre>
        ),
      };
    case "suspending":
      return {
        kind: "SUSPENDING",
        modifier: styles.rowSystem,
        subtitle: event.reason,
      };
    case "resumed":
      return {
        kind: "RESUMED",
        modifier: styles.rowSystem,
        subtitle: (
          <>
            from{" "}
            <Link
              to={`/sessions/${event.from_session_id}`}
              className={styles.link}
            >
              {event.from_session_id}
            </Link>{" "}
            <span className={styles.muted}>({event.source})</span>
          </>
        ),
      };
    case "closed":
      return { kind: "CLOSED", modifier: styles.rowSystem };
    case "system_event":
      return {
        kind: "SYSTEM",
        modifier: styles.rowSystem,
        subtitle: <code className={styles.code}>{event.kind.kind}</code>,
      };
    case "unknown":
      return {
        kind: "UNKNOWN",
        modifier: styles.rowSystem,
        subtitle: "unrecognized event type",
      };
  }
}

function eventTimestamp(event: SessionEvent): string | null {
  switch (event.type) {
    case "user_message":
    case "assistant_message":
    case "tool_use":
    case "suspending":
    case "resumed":
    case "closed":
    case "system_event":
      return event.timestamp;
    case "unknown":
      return null;
  }
}

export function SessionEventsView({ sessionId }: SessionEventsViewProps) {
  const { data: events, isLoading, error } = useSessionEvents(sessionId);

  if (isLoading) {
    return (
      <div className={styles.center}>
        <Spinner size="md" />
      </div>
    );
  }

  if (error) {
    const message =
      error instanceof ApiError
        ? `Failed to load events: ${error.message}`
        : `Failed to load events: ${(error as Error).message}`;
    return <p className={styles.error}>{message}</p>;
  }

  if (!events || events.length === 0) {
    return (
      <p className={styles.empty} data-testid="session-events-empty">
        No events recorded for this session yet.
      </p>
    );
  }

  return (
    <div className={styles.list} data-testid="session-events-list">
      {events.map((event, i) => {
        const { kind, modifier, subtitle, body } = describeEvent(event);
        const ts = eventTimestamp(event);
        return (
          <div
            key={i}
            className={`${styles.row}${modifier ? ` ${modifier}` : ""}`}
            data-event-kind={kind.toLowerCase()}
          >
            <div className={styles.head}>
              <span className={styles.badge}>{kind}</span>
              {subtitle && <span className={styles.subtitle}>{subtitle}</span>}
              <span className={styles.spacer} />
              {ts && (
                <span className={styles.ts} title={formatTimestamp(ts)}>
                  {formatTimestamp(ts)}
                </span>
              )}
            </div>
            {body && <div className={styles.body}>{body}</div>}
          </div>
        );
      })}
    </div>
  );
}
