import type { ActorRef, Issue, Session, SessionEvent } from "@hydra/api";
import type { Store } from "./store.js";
import { appendSessionEvent, getSessionEventsFor } from "./routes/sessions.js";
import { appendIssueComment, getIssueCommentsFor } from "./routes/issues.js";

const DEFAULT_INTERVAL_MS = 2500;
const SESSIONS_COLLECTION = "sessions";
const ISSUES_COLLECTION = "issues";

const SYNTHETIC_COMMENT_ACTOR: ActorRef = {
  Authenticated: { actor_id: { User: { name: "alice" } } },
};

export interface SyntheticEventsHandle {
  stop: () => void;
}

export interface SyntheticEventsOptions {
  intervalMs?: number;
}

type SyntheticVariant = "tool_use" | "assistant_message";

const ROTATION: readonly SyntheticVariant[] = [
  "tool_use",
  "assistant_message",
];

function buildSyntheticEvent(variant: SyntheticVariant, tick: number): SessionEvent {
  const timestamp = new Date().toISOString();
  if (variant === "tool_use") {
    return {
      type: "tool_use",
      tool_name: "mock_tool",
      payload: { note: `[mock] synthetic tool_use #${tick}` },
      timestamp,
    };
  }
  return {
    type: "assistant_message",
    content: `[mock] synthetic assistant_message #${tick}`,
    timestamp,
  };
}

/**
 * Start a background loop that appends one synthetic `SessionEvent` per tick
 * to every session whose latest version has `status === "running"`. Each
 * append flows through `appendSessionEvent`, which emits a
 * `session_event_created` SSE event on the global `/v1/events` stream so the
 * dev UI sees the chat / events tabs live-update without a manual refetch.
 *
 * The same tick also picks one in-progress issue per cycle and appends a
 * synthetic comment, fanned out via `appendIssueComment`, so the new
 * `comment_created` SSE path gets live exercise in the dev UI as well.
 *
 * The rotation is keyed off the current per-session event-log length so the
 * output is deterministic given a fixed seed (important for integration snapshots).
 *
 * Returns a handle whose `stop()` is idempotent: it clears the interval and
 * flips a per-handle `stopped` flag that causes any tick already queued by
 * the event loop to no-op when it runs.
 */
export function startSyntheticEvents(
  store: Store,
  options: SyntheticEventsOptions = {},
): SyntheticEventsHandle {
  const intervalMs = options.intervalMs ?? DEFAULT_INTERVAL_MS;
  let stopped = false;
  let commentTick = 0;
  const timer = setInterval(() => {
    if (stopped) return;
    const items = store.list<Session>(SESSIONS_COLLECTION, false);
    for (const { id, entry } of items) {
      if (entry.data.status !== "running") continue;
      const currentLength = getSessionEventsFor(id).length;
      const variant = ROTATION[currentLength % ROTATION.length];
      const event = buildSyntheticEvent(variant, currentLength + 1);
      appendSessionEvent(store, id, event);
    }

    // Round-robin one synthetic comment per tick across in-progress issues so
    // every running issue eventually gets a `comment_created` event without
    // flooding the stream when many issues are open at once.
    const issues = store
      .list<Issue>(ISSUES_COLLECTION, false)
      .filter(({ entry }) => entry.data.status?.key === "in-progress");
    if (issues.length > 0) {
      const target = issues[commentTick % issues.length];
      commentTick += 1;
      const seqHint = getIssueCommentsFor(target.id).length + 1;
      appendIssueComment(
        store,
        target.id,
        `[mock] synthetic comment #${seqHint}`,
        SYNTHETIC_COMMENT_ACTOR,
      );
    }
  }, intervalMs);

  // Don't keep the Node event loop alive purely because of this timer; the
  // HTTP server's `listen` socket is what should anchor the process.
  if (typeof timer.unref === "function") {
    timer.unref();
  }

  return {
    stop: () => {
      if (stopped) return;
      stopped = true;
      clearInterval(timer);
    },
  };
}
