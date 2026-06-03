import type { ConversationStatus, SessionEvent } from "@hydra/api";
import type { JsonValue } from "@hydra/api";
import { descriptionSnippet } from "../../utils/text";

export type ActivityCategory =
  | "think"
  | "read"
  | "edit"
  | "run"
  | "submit"
  | "error"
  | "done";

export type ActivityState = "live" | "done" | "error";

export interface ActivityStep {
  category: ActivityCategory;
  /**
   * Stable, human-readable label for the step (e.g. "Running command",
   * "Thinking…"). Renderers fall back to this when no `detail` is available.
   */
  verb: string;
  /**
   * Truncated `payload.description` from the tool call. Renderers prefer this
   * over `verb` when set, since it summarises the call in active voice.
   * `null` when the tool call has no usable description.
   */
  detail: string | null;
  /**
   * Raw tool name surfaced as inline code by renderers when no friendly
   * `verb` exists for the tool (i.e. the "Using <tool_name>" fallback).
   * `null` whenever `verb` or `detail` is sufficient on its own.
   */
  toolName: string | null;
  /** Step start time as epoch milliseconds. */
  startTs: number;
  /**
   * Step end time as epoch milliseconds; `null` while the step is the active
   * (in-flight) one and the agent has not yet moved on.
   */
  endTs: number | null;
}

export interface ActivityRun {
  /** Tool-use steps observed since the last `user_message`, in event order. */
  steps: ActivityStep[];
  /**
   * Step that should currently be displayed by the indicator. `null` when the
   * indicator should be hidden (terminal state, no in-flight activity, etc.).
   */
  current: ActivityStep | null;
  state: ActivityState;
  /**
   * Start of the current run as epoch milliseconds — the timestamp of the
   * last `user_message` (kick-off), falling back to the first step's start
   * when no user message is present. `0` when there's nothing to time.
   */
  startedAt: number;
}

/** Max length used when truncating a tool-call description for display. */
export const TOOL_DESCRIPTION_MAX_CHARS = 100;

/**
 * Tool-name → human label table. Kept as a plain record so it's trivial to
 * extend as Claude grows new tool names without touching the derivation logic.
 * Tool-name matching is case-sensitive against the names emitted by Claude.
 */
export const TOOL_LABELS: Record<string, string> = {
  Bash: "Running command",
  Read: "Reading file",
  Edit: "Editing file",
  Write: "Editing file",
  NotebookEdit: "Editing file",
  Grep: "Searching code",
  Glob: "Searching files",
  WebFetch: "Fetching from web",
  WebSearch: "Searching the web",
  Task: "Delegating subtask",
  Agent: "Delegating subtask",
  TodoWrite: "Updating plan",
};

const READ_TOOLS = new Set(["Read", "Grep", "Glob", "WebFetch", "WebSearch"]);
const EDIT_TOOLS = new Set(["Edit", "Write", "NotebookEdit"]);
const RUN_TOOLS = new Set(["Bash"]);
const THINK_TOOLS = new Set(["TodoWrite"]);
const SUBMIT_TOOLS = new Set(["Task", "Agent"]);

/**
 * Map a Claude tool name to a colour-coding category. Unknown tools fall back
 * to `run` so the indicator matches the existing amber "working" affordance.
 */
export function categorizeTool(toolName: string): ActivityCategory {
  if (READ_TOOLS.has(toolName)) return "read";
  if (EDIT_TOOLS.has(toolName)) return "edit";
  if (RUN_TOOLS.has(toolName)) return "run";
  if (THINK_TOOLS.has(toolName)) return "think";
  if (SUBMIT_TOOLS.has(toolName)) return "submit";
  return "run";
}

function extractToolDescription(payload: JsonValue): string | null {
  if (payload === null || typeof payload !== "object" || Array.isArray(payload)) {
    return null;
  }
  const desc = (payload as { [key: string]: JsonValue }).description;
  if (typeof desc !== "string") return null;
  const trimmed = desc.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function parseTs(ts: string): number {
  const n = Date.parse(ts);
  return Number.isNaN(n) ? 0 : n;
}

function eventTs(ev: SessionEvent): number {
  if (ev.type === "unknown") return 0;
  return parseTs(ev.timestamp);
}

function lastUserMessageIndex(events: readonly SessionEvent[]): number {
  for (let i = events.length - 1; i >= 0; i--) {
    if (events[i].type === "user_message") return i;
  }
  return -1;
}

function buildToolStep(
  ev: Extract<SessionEvent, { type: "tool_use" }>,
): ActivityStep {
  const description = extractToolDescription(ev.payload);
  const detail = description
    ? descriptionSnippet(description, TOOL_DESCRIPTION_MAX_CHARS)
    : null;
  const label = TOOL_LABELS[ev.tool_name];
  const verb = label ?? "Using";
  // toolName is the inline-code fallback — only meaningful when neither a
  // friendly `verb` nor a `detail` description is available to render.
  const toolName = !label && !detail ? ev.tool_name : null;
  return {
    category: categorizeTool(ev.tool_name),
    verb,
    detail,
    toolName,
    startTs: parseTs(ev.timestamp),
    endTs: null,
  };
}

/**
 * Derive the activity run from the (already merged with optimistic)
 * `SessionEvent` log and the conversation's status.
 *
 * Replaces the previous tail-only `deriveActivityStatus` so the indicator can
 * be evolved into a step-by-step feed in a follow-up PR ([[i-ieoefegy]]).
 *
 * - `steps` — one entry per `tool_use` since the last `user_message`. Each
 *   step's `endTs` is set from the next event's timestamp; the active (last)
 *   step has `endTs = null` while the agent is still in flight.
 * - `current` — the step the indicator should render right now. Synthesised
 *   for the `user_message`/`resumed` tails so the existing "Thinking…" /
 *   "Resuming session…" UI continues to work unchanged.
 * - `state` — `'done'` when the conversation is closed or the tail is
 *   `assistant_message`/`closed`; otherwise `'live'`. `'error'` is reserved
 *   for a future failure signal (see PR description).
 * - `startedAt` — kick-off timestamp of the current run (the last
 *   `user_message`'s timestamp, falling back to the first step's start).
 */
export function deriveActivitySteps(
  events: readonly SessionEvent[],
  conversationStatus: ConversationStatus,
  now: number = Date.now(),
): ActivityRun {
  const conversationClosed = conversationStatus === "closed";

  const userIdx = lastUserMessageIndex(events);
  const sliceStart = userIdx === -1 ? 0 : userIdx;
  const slice = events.slice(sliceStart);

  const steps: ActivityStep[] = [];
  const toolUseSliceIdx: number[] = [];
  for (let i = 0; i < slice.length; i++) {
    const ev = slice[i];
    if (ev.type === "tool_use") {
      steps.push(buildToolStep(ev));
      toolUseSliceIdx.push(i);
    }
  }
  for (let i = 0; i < steps.length; i++) {
    const sliceIdx = toolUseSliceIdx[i];
    const nextEv = slice[sliceIdx + 1];
    if (nextEv) {
      steps[i].endTs = eventTs(nextEv);
    }
  }

  const startedAt =
    userIdx !== -1
      ? eventTs(events[userIdx])
      : steps.length > 0
        ? steps[0].startTs
        : 0;

  const tail = events.length > 0 ? events[events.length - 1] : null;
  const isTerminalTail =
    tail !== null && (tail.type === "closed" || tail.type === "assistant_message");
  const state: ActivityState =
    conversationClosed || isTerminalTail ? "done" : "live";

  let current: ActivityStep | null = null;
  if (state === "done") {
    for (const step of steps) {
      if (step.endTs === null) {
        const tailTs = tail ? eventTs(tail) : 0;
        step.endTs = tailTs > step.startTs ? tailTs : now;
      }
    }
  } else if (tail !== null) {
    switch (tail.type) {
      case "tool_use":
        current = steps[steps.length - 1] ?? null;
        break;
      case "user_message":
        current = {
          category: "think",
          verb: "Thinking…",
          detail: null,
          toolName: null,
          startTs: parseTs(tail.timestamp),
          endTs: null,
        };
        break;
      case "resumed":
        current = {
          category: "think",
          verb: "Resuming session…",
          detail: null,
          toolName: null,
          startTs: parseTs(tail.timestamp),
          endTs: null,
        };
        break;
      case "suspending":
      case "unknown":
        current = null;
        break;
      case "assistant_message":
      case "closed":
        current = null;
        break;
    }
  }

  return { steps, current, state, startedAt };
}
