import type { Session, SessionEvent } from "@hydra/api";
import type { Store } from "./store.js";
import { appendSessionEvent, getSessionEventsFor } from "./routes/sessions.js";

const DEFAULT_INTERVAL_MS = 2500;
const SESSIONS_COLLECTION = "sessions";

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
 * The rotation is keyed off the current per-session event-log length so the
 * output is deterministic given a fixed seed (important for e2e snapshots).
 *
 * Returns a handle whose `stop()` is idempotent and uses a generation counter
 * so any in-flight ticks scheduled before `stop()` no-op rather than racing
 * with a subsequent `start`.
 */
export function startSyntheticEvents(
  store: Store,
  options: SyntheticEventsOptions = {},
): SyntheticEventsHandle {
  const intervalMs = options.intervalMs ?? DEFAULT_INTERVAL_MS;
  let stopped = false;
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
