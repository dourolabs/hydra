import type { ConversationStatus, SessionEvent } from "@hydra/api";
import type { JsonValue } from "@hydra/api";
import { descriptionSnippet } from "../../utils/text";

export interface ActivityStatus {
  /**
   * Human-readable label for the indicator. Renderers display this as-is and
   * append `toolName` (if present) as inline code — gives the renderer the
   * raw tool name without forcing it to parse markdown out of `text`.
   */
  text: string;
  /**
   * Only populated for the fallback `Using <tool_name>` branch; the renderer
   * should append it after `text` styled as inline code.
   */
  toolName?: string;
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

/**
 * Pull a human-readable `description` string out of a tool-call payload, if
 * one is present. Many Claude tools (e.g. `Bash`, `Agent`) ship a short
 * `description` in their input that summarises the call in active voice —
 * far more informative than the bare tool name.
 *
 * Returns `null` when the payload is not an object, lacks a string
 * `description`, or carries an empty/whitespace-only description.
 */
function extractToolDescription(payload: JsonValue): string | null {
  if (payload === null || typeof payload !== "object" || Array.isArray(payload)) {
    return null;
  }
  const desc = (payload as { [key: string]: JsonValue }).description;
  if (typeof desc !== "string") return null;
  const trimmed = desc.trim();
  return trimmed.length > 0 ? trimmed : null;
}

/**
 * Derive what the transient activity indicator should display based on the
 * tail of the (already merged with optimistic) `SessionEvent` log and the
 * conversation's status. Returns `null` to mean "hide the indicator".
 *
 * The function is intentionally pure — no hooks, no React imports — so it
 * can be unit-tested table-style and consumed from any render surface.
 *
 * Mapping (see issue [[i-hgmaqhor]]):
 * - `UserMessage` tail → `Thinking…`
 * - `ToolUse` tail →
 *     1. payload `description` field (truncated) when present
 *        (per [[i-jxamlakh]]: tools like `Bash`/`Agent` carry a human-readable
 *        description and surfacing it is more informative than the tool name),
 *     2. otherwise tool-specific label from `TOOL_LABELS`,
 *     3. otherwise fallback `Using <tool_name>` (toolName exposed separately
 *        for inline-code render).
 * - `Resumed` tail → `Resuming session…`
 * - `AssistantMessage` / `Suspending` / `Closed` / `Unknown` tail → `null`
 *   (the next assistant message is the visible signal; system-event tails
 *   don't need a live indicator).
 * - Empty events or `Closed` conversation → `null`.
 */
export function deriveActivityStatus(
  events: readonly SessionEvent[],
  conversationStatus: ConversationStatus,
): ActivityStatus | null {
  if (conversationStatus === "closed") return null;
  if (events.length === 0) return null;

  const tail = events[events.length - 1];
  switch (tail.type) {
    case "user_message":
      return { text: "Thinking…" };
    case "tool_use": {
      const description = extractToolDescription(tail.payload);
      if (description) {
        return { text: descriptionSnippet(description, TOOL_DESCRIPTION_MAX_CHARS) };
      }
      const label = TOOL_LABELS[tail.tool_name];
      if (label) return { text: label };
      return { text: "Using", toolName: tail.tool_name };
    }
    case "resumed":
      return { text: "Resuming session…" };
    case "assistant_message":
    case "suspending":
    case "closed":
    case "unknown":
      return null;
  }
}
